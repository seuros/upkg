use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::types::{BuildPlan, Error};
use tokio::fs;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;

use super::environment::build_env;
use super::formula_parser::{
    InstallAction, InstallPlan, InstallTarget, parse_supported_install_plan,
};
use super::source::download_and_extract_source;

const SHIM_RUBY: &str = include_str!("shim.rb");

pub struct BuildExecutor {
    prefix: PathBuf,
    work_root: PathBuf,
}

impl BuildExecutor {
    pub fn new(prefix: PathBuf, root: PathBuf) -> Self {
        let work_root = root.join("cache").join("build");
        Self { prefix, work_root }
    }

    pub async fn execute(
        &self,
        plan: &BuildPlan,
        formula_rb_path: &Path,
        installed_deps: &HashMap<String, DepInfo>,
    ) -> Result<(), Error> {
        let work_dir = self.work_root.join(&plan.formula_name);
        self.prepare_work_dir(&work_dir).await?;

        let source_root = download_and_extract_source(
            &plan.source_url,
            plan.source_checksum.as_deref(),
            &work_dir,
        )
        .await?;

        let shim_path = work_dir.join("upkg_shim.rb");
        fs::write(&shim_path, SHIM_RUBY)
            .await
            .map_err(|e| Error::FileError {
                message: format!("failed to write ruby shim: {e}"),
            })?;

        fs::create_dir_all(&plan.cellar_path)
            .await
            .map_err(|e| Error::FileError {
                message: format!("failed to create cellar directory: {e}"),
            })?;

        let formula_source =
            fs::read_to_string(formula_rb_path)
                .await
                .map_err(|e| Error::FileError {
                    message: format!("failed to read formula source: {e}"),
                })?;
        if let Some(native_plan) = parse_supported_install_plan(&formula_source)? {
            execute_native_install_plan(plan, &source_root, &native_plan).await?;
            self.cleanup_work_dir(&work_dir).await;
            return Ok(());
        }

        let mut env = build_env(plan, &self.prefix);
        env.insert(
            "UPKG_FORMULA_FILE".into(),
            formula_rb_path.display().to_string(),
        );

        let deps_json = serde_json::to_string(installed_deps).unwrap_or_else(|_| "{}".into());
        env.insert("UPKG_INSTALLED_DEPS".into(), deps_json);

        let ruby = find_ruby().await?;
        run_build(&ruby, &shim_path, &source_root, &env).await?;

        self.cleanup_work_dir(&work_dir).await;
        Ok(())
    }

    async fn prepare_work_dir(&self, work_dir: &Path) -> Result<(), Error> {
        if work_dir.exists() {
            let _ = fs::remove_dir_all(work_dir).await;
        }
        fs::create_dir_all(work_dir)
            .await
            .map_err(|e| Error::FileError {
                message: format!("failed to create work directory: {e}"),
            })
    }

    async fn cleanup_work_dir(&self, work_dir: &Path) {
        let _ = fs::remove_dir_all(work_dir).await;
    }
}

async fn execute_native_install_plan(
    plan: &BuildPlan,
    source_root: &Path,
    native_plan: &InstallPlan,
) -> Result<(), Error> {
    for action in &native_plan.actions {
        match action {
            InstallAction::Move {
                sources,
                destination,
            }
            | InstallAction::Install {
                destination,
                sources,
            } => {
                let destination = install_destination(plan, *destination);
                install_sources(source_root, &destination, sources).await?;
            }
        }
    }

    Ok(())
}

async fn install_sources(
    source_root: &Path,
    destination: &Path,
    sources: &[String],
) -> Result<(), Error> {
    fs::create_dir_all(destination)
        .await
        .map_err(|e| Error::FileError {
            message: format!(
                "failed to create install destination '{}': {e}",
                destination.display()
            ),
        })?;

    for source in sources {
        let source_path = source_root.join(source);
        let metadata = fs::symlink_metadata(&source_path)
            .await
            .map_err(|e| Error::FileError {
                message: format!(
                    "failed to read source '{}' for native install plan: {e}",
                    source_path.display()
                ),
            })?;

        let file_name = source_path.file_name().ok_or_else(|| Error::FileError {
            message: format!(
                "native install source '{}' has no basename",
                source_path.display()
            ),
        })?;

        let target_path = destination.join(file_name);
        move_path(&source_path, &target_path, metadata.is_dir())?;
    }

    Ok(())
}

fn move_path(source: &Path, target: &Path, is_dir: bool) -> Result<(), Error> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::FileError {
            message: format!("failed to create target parent '{}': {e}", parent.display()),
        })?;
    }

    if is_dir {
        if target.exists() {
            let entries = std::fs::read_dir(source).map_err(|e| Error::FileError {
                message: format!(
                    "failed to read source directory '{}': {e}",
                    source.display()
                ),
            })?;
            for entry in entries {
                let entry = entry.map_err(|e| Error::FileError {
                    message: format!(
                        "failed to iterate source directory '{}': {e}",
                        source.display()
                    ),
                })?;
                let child_source = entry.path();
                let child_target = target.join(entry.file_name());
                let child_is_dir = entry
                    .file_type()
                    .map_err(|e| Error::FileError {
                        message: format!(
                            "failed to inspect source entry '{}': {e}",
                            child_source.display()
                        ),
                    })?
                    .is_dir();
                move_path(&child_source, &child_target, child_is_dir)?;
            }

            std::fs::remove_dir(source).map_err(|e| Error::FileError {
                message: format!(
                    "failed to remove source directory '{}' after move: {e}",
                    source.display()
                ),
            })?;
            return Ok(());
        }
    }

    std::fs::rename(source, target).map_err(|e| Error::FileError {
        message: format!(
            "failed to move '{}' to '{}': {e}",
            source.display(),
            target.display()
        ),
    })
}

fn install_destination(plan: &BuildPlan, target: InstallTarget) -> PathBuf {
    match target {
        InstallTarget::Prefix => plan.cellar_path.clone(),
        InstallTarget::Bin => plan.cellar_path.join("bin"),
        InstallTarget::Sbin => plan.cellar_path.join("sbin"),
        InstallTarget::Lib => plan.cellar_path.join("lib"),
        InstallTarget::Libexec => plan.cellar_path.join("libexec"),
        InstallTarget::Include => plan.cellar_path.join("include"),
        InstallTarget::Share => plan.cellar_path.join("share"),
        InstallTarget::Man => plan.cellar_path.join("share").join("man"),
        InstallTarget::Man1 => plan.cellar_path.join("share").join("man").join("man1"),
        InstallTarget::Man2 => plan.cellar_path.join("share").join("man").join("man2"),
        InstallTarget::Man3 => plan.cellar_path.join("share").join("man").join("man3"),
        InstallTarget::Man4 => plan.cellar_path.join("share").join("man").join("man4"),
        InstallTarget::Man5 => plan.cellar_path.join("share").join("man").join("man5"),
        InstallTarget::Man6 => plan.cellar_path.join("share").join("man").join("man6"),
        InstallTarget::Man7 => plan.cellar_path.join("share").join("man").join("man7"),
        InstallTarget::Man8 => plan.cellar_path.join("share").join("man").join("man8"),
        InstallTarget::Doc => plan
            .cellar_path
            .join("share")
            .join("doc")
            .join(&plan.formula_name),
        InstallTarget::Info => plan.cellar_path.join("share").join("info"),
        InstallTarget::Pkgshare => plan.cellar_path.join("share").join(&plan.formula_name),
        InstallTarget::BashCompletion => plan.cellar_path.join("etc").join("bash_completion.d"),
        InstallTarget::ZshCompletion => plan
            .cellar_path
            .join("share")
            .join("zsh")
            .join("site-functions"),
        InstallTarget::FishCompletion => plan
            .cellar_path
            .join("share")
            .join("fish")
            .join("vendor_completions.d"),
        InstallTarget::Elisp => plan
            .cellar_path
            .join("share")
            .join("emacs")
            .join("site-lisp")
            .join(&plan.formula_name),
        InstallTarget::Frameworks => plan.cellar_path.join("Frameworks"),
        InstallTarget::Kext => plan.cellar_path.join("Library").join("Extensions"),
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DepInfo {
    pub cellar_path: String,
}

async fn find_ruby() -> Result<PathBuf, Error> {
    for candidate in ["ruby", "/usr/bin/ruby"] {
        let result = Command::new(candidate).arg("--version").output().await;

        if let Ok(output) = result
            && output.status.success()
        {
            return Ok(PathBuf::from(candidate));
        }
    }

    Err(Error::ExecutionError {
        message: "ruby not found — required for building from source".into(),
    })
}

async fn run_build(
    ruby: &Path,
    shim_path: &Path,
    source_root: &Path,
    env: &HashMap<String, String>,
) -> Result<(), Error> {
    let mut child = Command::new(ruby)
        .arg(shim_path)
        .current_dir(source_root)
        .envs(env)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Error::ExecutionError {
            message: format!("failed to execute ruby shim: {e}"),
        })?;

    let stdout = child.stdout.take().ok_or_else(|| Error::ExecutionError {
        message: "failed to capture ruby shim stdout".to_string(),
    })?;
    let stderr = child.stderr.take().ok_or_else(|| Error::ExecutionError {
        message: "failed to capture ruby shim stderr".to_string(),
    })?;

    let stdout_task = tokio::spawn(stream_output_and_capture_tail(stdout, false));
    let stderr_task = tokio::spawn(stream_output_and_capture_tail(stderr, true));

    let status = child.wait().await.map_err(|e| Error::ExecutionError {
        message: format!("failed waiting for ruby shim: {e}"),
    })?;

    let stdout_tail = stdout_task
        .await
        .map_err(|e| Error::ExecutionError {
            message: format!("failed to join stdout task: {e}"),
        })?
        .map_err(|e| Error::ExecutionError {
            message: format!("failed reading stdout: {e}"),
        })?;
    let stderr_tail = stderr_task
        .await
        .map_err(|e| Error::ExecutionError {
            message: format!("failed to join stderr task: {e}"),
        })?
        .map_err(|e| Error::ExecutionError {
            message: format!("failed reading stderr: {e}"),
        })?;

    if !status.success() {
        let mut msg = format!("source build failed (exit code: {:?})", status.code());
        let tail = if !stderr_tail.is_empty() {
            stderr_tail
        } else {
            stdout_tail
        };
        if !tail.is_empty() {
            msg.push('\n');
            msg.push_str(&tail.join("\n"));
        }
        return Err(Error::ExecutionError { message: msg });
    }

    Ok(())
}

async fn stream_output_and_capture_tail<R>(
    reader: R,
    stderr: bool,
) -> Result<Vec<String>, std::io::Error>
where
    R: AsyncRead + Unpin,
{
    const TAIL_LINES: usize = 40;
    let mut tail = VecDeque::with_capacity(TAIL_LINES);
    let mut lines = BufReader::new(reader).lines();

    while let Some(line) = lines.next_line().await? {
        if stderr {
            eprintln!("{line}");
        } else {
            println!("{line}");
        }

        if tail.len() == TAIL_LINES {
            tail.pop_front();
        }
        tail.push_back(line);
    }

    Ok(tail.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::BuildSystem;

    fn test_build_plan(prefix: &Path) -> BuildPlan {
        let cellar_path = prefix.join("Cellar").join("foo").join("1.0.0");
        BuildPlan {
            formula_name: "foo".to_string(),
            version: "1.0.0".to_string(),
            source_url: "https://example.com/foo-1.0.0.tar.gz".to_string(),
            source_checksum: None,
            ruby_source_path: None,
            build_dependencies: Vec::new(),
            runtime_dependencies: Vec::new(),
            detected_system: BuildSystem::RubyFormula,
            prefix: prefix.to_path_buf(),
            cellar_path,
        }
    }

    #[tokio::test]
    async fn native_install_plan_moves_supported_targets() {
        let tmp = tempfile::tempdir().unwrap();
        let source_root = tmp.path().join("source");
        std::fs::create_dir_all(source_root.join("themes")).unwrap();
        std::fs::create_dir_all(source_root.join("build")).unwrap();
        std::fs::write(source_root.join("themes/default.omp.json"), "{}").unwrap();
        std::fs::write(source_root.join("build/foo"), "#!/bin/sh\n").unwrap();
        std::fs::write(source_root.join("README.md"), "readme").unwrap();

        let prefix = tmp.path().join("prefix");
        let plan = test_build_plan(&prefix);
        let native_plan = InstallPlan {
            actions: vec![
                InstallAction::Move {
                    sources: vec!["themes".to_string()],
                    destination: InstallTarget::Prefix,
                },
                InstallAction::Install {
                    destination: InstallTarget::Bin,
                    sources: vec!["build/foo".to_string()],
                },
                InstallAction::Install {
                    destination: InstallTarget::Doc,
                    sources: vec!["README.md".to_string()],
                },
            ],
        };

        execute_native_install_plan(&plan, &source_root, &native_plan)
            .await
            .unwrap();

        assert!(
            plan.cellar_path
                .join("themes")
                .join("default.omp.json")
                .exists()
        );
        assert!(plan.cellar_path.join("bin").join("foo").exists());
        assert!(
            plan.cellar_path
                .join("share")
                .join("doc")
                .join("foo")
                .join("README.md")
                .exists()
        );
    }

    #[tokio::test]
    async fn run_build_supports_mv_in_formula_install() {
        let Some(ruby) = find_ruby().await.ok() else {
            return;
        };

        let tmp = tempfile::tempdir().unwrap();
        let source_root = tmp.path().join("source");
        std::fs::create_dir_all(source_root.join("themes")).unwrap();
        std::fs::write(source_root.join("themes/default.omp.json"), "{}").unwrap();

        let shim_path = tmp.path().join("shim.rb");
        std::fs::write(&shim_path, SHIM_RUBY).unwrap();

        let formula_path = tmp.path().join("foo.rb");
        std::fs::write(
            &formula_path,
            r#"
class Foo < Formula
  def install
    mv "themes", prefix
  end
end
"#,
        )
        .unwrap();

        let prefix = tmp.path().join("prefix");
        let cellar = prefix.join("Cellar");
        std::fs::create_dir_all(&cellar).unwrap();

        let mut env = HashMap::new();
        env.insert("UPKG_PREFIX".to_string(), prefix.display().to_string());
        env.insert("UPKG_CELLAR".to_string(), cellar.display().to_string());
        env.insert("UPKG_FORMULA_NAME".to_string(), "foo".to_string());
        env.insert("UPKG_FORMULA_VERSION".to_string(), "1.0.0".to_string());
        env.insert(
            "UPKG_FORMULA_FILE".to_string(),
            formula_path.display().to_string(),
        );
        env.insert("UPKG_INSTALLED_DEPS".to_string(), "{}".to_string());

        run_build(&ruby, &shim_path, &source_root, &env)
            .await
            .unwrap();

        assert!(
            prefix
                .join("Cellar")
                .join("foo")
                .join("1.0.0")
                .join("themes")
                .join("default.omp.json")
                .exists()
        );
    }

    #[tokio::test]
    async fn run_build_includes_stderr_tail_in_error() {
        let Some(ruby) = find_ruby().await.ok() else {
            return;
        };

        let tmp = tempfile::tempdir().unwrap();
        let source_root = tmp.path().join("source");
        std::fs::create_dir_all(&source_root).unwrap();

        let shim_path = tmp.path().join("shim.rb");
        std::fs::write(&shim_path, SHIM_RUBY).unwrap();

        let formula_path = tmp.path().join("foo.rb");
        std::fs::write(
            &formula_path,
            r#"
class Foo < Formula
  def install
    system "sh", "-c", "echo boom-from-stderr 1>&2; exit 7"
  end
end
"#,
        )
        .unwrap();

        let prefix = tmp.path().join("prefix");
        let cellar = prefix.join("Cellar");
        std::fs::create_dir_all(&cellar).unwrap();

        let mut env = HashMap::new();
        env.insert("UPKG_PREFIX".to_string(), prefix.display().to_string());
        env.insert("UPKG_CELLAR".to_string(), cellar.display().to_string());
        env.insert("UPKG_FORMULA_NAME".to_string(), "foo".to_string());
        env.insert("UPKG_FORMULA_VERSION".to_string(), "1.0.0".to_string());
        env.insert(
            "UPKG_FORMULA_FILE".to_string(),
            formula_path.display().to_string(),
        );
        env.insert("UPKG_INSTALLED_DEPS".to_string(), "{}".to_string());

        let err = run_build(&ruby, &shim_path, &source_root, &env)
            .await
            .unwrap_err();

        let message = err.to_string();
        assert!(message.contains("source build failed"));
        assert!(message.contains("boom-from-stderr"));
    }
}
