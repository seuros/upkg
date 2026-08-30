use std::env;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HealthStatus {
    Ok,
    Warn,
    Fail,
}

impl HealthStatus {
    fn marker(self) -> &'static str {
        match self {
            Self::Ok => "✓",
            Self::Warn => "!",
            Self::Fail => "✗",
        }
    }
}

#[derive(Debug)]
struct HealthCheck {
    status: HealthStatus,
    label: &'static str,
    detail: String,
}

impl HealthCheck {
    fn ok(label: &'static str, detail: impl Into<String>) -> Self {
        Self {
            status: HealthStatus::Ok,
            label,
            detail: detail.into(),
        }
    }

    fn warn(label: &'static str, detail: impl Into<String>) -> Self {
        Self {
            status: HealthStatus::Warn,
            label,
            detail: detail.into(),
        }
    }

    fn fail(label: &'static str, detail: impl Into<String>) -> Self {
        Self {
            status: HealthStatus::Fail,
            label,
            detail: detail.into(),
        }
    }
}

pub fn run() -> bool {
    let checks = collect_checks();
    let healthy = checks
        .iter()
        .all(|check| check.status != HealthStatus::Fail);

    println!("upkg shaman");
    println!("os: {}", env::consts::OS);
    println!();

    for check in checks {
        println!(
            "{} {:<18} {}",
            check.status.marker(),
            check.label,
            check.detail
        );
    }

    println!();
    if healthy {
        println!("diagnosis: upkg looks healthy");
    } else {
        println!("diagnosis: upkg needs attention");
    }

    healthy
}

fn collect_checks() -> Vec<HealthCheck> {
    let mut checks = Vec::new();
    checks.push(path_check());
    checks.extend(folder_checks());
    checks.push(backend_check());
    checks
}

fn path_check() -> HealthCheck {
    match env::current_exe() {
        Ok(exe) => {
            let command = exe
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("upkg");
            if find_command(command).is_some() {
                HealthCheck::ok("path", format!("{command} is available in PATH"))
            } else {
                HealthCheck::fail(
                    "path",
                    format!("{command} is not available in PATH; run `upkg init` or update PATH"),
                )
            }
        }
        Err(err) => HealthCheck::warn(
            "path",
            format!("could not inspect current executable: {err}"),
        ),
    }
}

fn folder_checks() -> Vec<HealthCheck> {
    let mut checks = Vec::new();

    let upkg_dir = upkg_dir();
    checks.push(directory_check("upkg dir", &upkg_dir));

    let upkg_bin = env::var_os("UPKG_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| upkg_dir.join("bin"));
    checks.push(directory_check("upkg bin", &upkg_bin));

    #[cfg(target_os = "macos")]
    {
        let context = crate::types::Context::from_defaults();
        let root = env::var_os("UPKG_ROOT")
            .map(PathBuf::from)
            .unwrap_or(context.paths.root);
        let prefix = env::var_os("UPKG_PREFIX")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.clone());

        checks.push(directory_check("root", &root));
        checks.push(directory_check("prefix", &prefix));

        for dir in macos_managed_dirs(&root, &prefix) {
            checks.push(directory_check("managed dir", &dir));
        }
    }

    checks
}

fn directory_check(label: &'static str, path: &Path) -> HealthCheck {
    if path.is_dir() {
        HealthCheck::ok(label, path.display().to_string())
    } else {
        HealthCheck::fail(label, format!("{} is missing", path.display()))
    }
}

fn upkg_dir() -> PathBuf {
    if let Some(path) = env::var_os("UPKG_DIR") {
        return PathBuf::from(path);
    }

    home_dir()
        .map(|home| home.join(".upkg"))
        .unwrap_or_else(|| PathBuf::from(".upkg"))
}

fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        env::var_os("USERPROFILE").map(PathBuf::from).or_else(|| {
            match (env::var_os("HOMEDRIVE"), env::var_os("HOMEPATH")) {
                (Some(drive), Some(path)) => {
                    let mut value = PathBuf::from(drive);
                    value.push(path);
                    Some(value)
                }
                _ => None,
            }
        })
    }

    #[cfg(not(windows))]
    {
        env::var_os("HOME").map(PathBuf::from)
    }
}

#[cfg(target_os = "macos")]
fn macos_managed_dirs(root: &Path, prefix: &Path) -> Vec<PathBuf> {
    const PREFIX_MANAGED_DIRS: &[&str] = &[
        "bin",
        "sbin",
        "Cellar",
        "opt",
        "lib",
        "libexec",
        "cli-plugins",
        "include",
        "share",
        "etc",
    ];

    let mut dirs = vec![
        root.join("store"),
        root.join("cache"),
        root.join("locks"),
        root.join("db"),
    ];
    dirs.extend(PREFIX_MANAGED_DIRS.iter().map(|dir| prefix.join(dir)));
    dirs
}

#[cfg(target_os = "macos")]
fn backend_check() -> HealthCheck {
    HealthCheck::ok("backend", "built-in macOS Homebrew-compatible backend")
}

#[cfg(not(target_os = "macos"))]
fn backend_check() -> HealthCheck {
    match crate::backend::Backend::detect() {
        Ok(backend) => HealthCheck::ok("backend", format!("{} detected", backend.name())),
        Err(err) => HealthCheck::fail("backend", err.to_string()),
    }
}

fn find_command(name: &str) -> Option<PathBuf> {
    if name.contains(std::path::MAIN_SEPARATOR) {
        let path = PathBuf::from(name);
        return path.is_file().then_some(path);
    }

    let paths = env::var_os("PATH")?;
    env::split_paths(&paths).find_map(|path| {
        let candidate = path.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }

        #[cfg(windows)]
        {
            for ext in ["exe", "cmd", "bat"] {
                let candidate = path.join(format!("{name}.{ext}"));
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }

        None
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_check_fails_for_missing_directory() {
        let check = directory_check("test", Path::new("/definitely/missing/upkg/path"));
        assert_eq!(check.status, HealthStatus::Fail);
        assert!(check.detail.contains("missing"));
    }

    #[test]
    fn find_command_finds_rustc() {
        assert!(find_command("rustc").is_some());
    }
}
