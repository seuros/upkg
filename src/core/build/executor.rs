use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::types::{BuildPlan, Error};
use tokio::fs;

use super::formula_parser::{
    InstallAction, InstallPlan, InstallSource, InstallTarget, parse_supported_install_plan,
};
use super::source::download_and_extract_source;

pub struct BuildExecutor {
    work_root: PathBuf,
}

impl BuildExecutor {
    pub fn new(_prefix: PathBuf, root: PathBuf) -> Self {
        let work_root = root.join("cache").join("build");
        Self { work_root }
    }

    pub async fn execute(
        &self,
        plan: &BuildPlan,
        formula_rb_path: &Path,
        _installed_deps: &HashMap<String, DepInfo>,
    ) -> Result<(), Error> {
        // Reject unsupported formulas before downloading anything or creating
        // the keg, so a refusal leaves no trace in the cellar.
        let formula_source =
            fs::read_to_string(formula_rb_path)
                .await
                .map_err(|e| Error::FileError {
                    message: format!("failed to read formula source: {e}"),
                })?;
        let native_plan =
            parse_supported_install_plan(&formula_source)?.ok_or_else(|| {
                Error::UnsupportedFormula {
                    name: plan.formula_name.clone(),
                    reason: "formula install block uses unsupported Homebrew DSL; Ruby fallback has been removed".to_string(),
                }
            })?;

        let work_dir = self.work_root.join(&plan.formula_name);
        self.prepare_work_dir(&work_dir).await?;

        let source_root = download_and_extract_source(
            &plan.source_url,
            plan.source_checksum.as_deref(),
            &work_dir,
        )
        .await?;

        fs::create_dir_all(&plan.cellar_path)
            .await
            .map_err(|e| Error::FileError {
                message: format!("failed to create cellar directory: {e}"),
            })?;

        execute_native_install_plan(plan, &source_root, &native_plan).await?;
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
            } => {
                let target = *destination;
                let destination = install_destination(plan, target);
                let install_sources = sources
                    .iter()
                    .map(|source| InstallSource {
                        source: source.clone(),
                        target_name: None,
                    })
                    .collect::<Vec<_>>();
                install_sources_into(
                    source_root,
                    &destination,
                    &install_sources,
                    is_executable_target(target),
                )
                .await?;
            }
            InstallAction::Install {
                destination,
                sources,
            } => {
                let target = *destination;
                let destination = install_destination(plan, target);
                install_sources_into(
                    source_root,
                    &destination,
                    sources,
                    is_executable_target(target),
                )
                .await?;
            }
        }
    }

    Ok(())
}

async fn install_sources_into(
    source_root: &Path,
    destination: &Path,
    sources: &[InstallSource],
    executable_target: bool,
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
        let source_path = source_root.join(&source.source);
        let metadata = fs::symlink_metadata(&source_path)
            .await
            .map_err(|e| Error::FileError {
                message: format!(
                    "failed to read source '{}' for native install plan: {e}",
                    source_path.display()
                ),
            })?;

        let default_file_name = source_path.file_name().ok_or_else(|| Error::FileError {
            message: format!(
                "native install source '{}' has no basename",
                source_path.display()
            ),
        })?;

        let target_path = match &source.target_name {
            Some(target_name) => destination.join(target_name),
            None => destination.join(default_file_name),
        };
        move_path(&source_path, &target_path, metadata.is_dir())?;
        if executable_target {
            ensure_executable(&target_path)?;
        }
    }

    Ok(())
}

fn is_executable_target(target: InstallTarget) -> bool {
    matches!(target, InstallTarget::Bin | InstallTarget::Sbin)
}

fn ensure_executable(path: &Path) -> Result<(), Error> {
    let metadata = std::fs::metadata(path).map_err(|e| Error::FileError {
        message: format!("failed to read installed path '{}': {e}", path.display()),
    })?;
    if metadata.is_dir() {
        return Ok(());
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = metadata.permissions();
        let mode = permissions.mode();
        let executable_bits = (mode & 0o444) >> 2;
        permissions.set_mode(mode | executable_bits);
        std::fs::set_permissions(path, permissions).map_err(|e| Error::FileError {
            message: format!(
                "failed to mark installed path '{}' executable: {e}",
                path.display()
            ),
        })?;
    }

    Ok(())
}

fn move_path(source: &Path, target: &Path, is_dir: bool) -> Result<(), Error> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::FileError {
            message: format!("failed to create target parent '{}': {e}", parent.display()),
        })?;
    }

    if is_dir && target.exists() {
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

#[cfg(test)]
#[path = "executor/tests.rs"]
mod tests;
