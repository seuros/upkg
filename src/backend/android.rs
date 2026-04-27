use crate::backend::{CommandSpec, command_exists};
use crate::error::UpkgError;

pub enum AndroidManager {
    Pkg,
    Apt,
}

pub fn detect() -> Result<AndroidManager, UpkgError> {
    if command_exists("pkg") {
        return Ok(AndroidManager::Pkg);
    }

    if command_exists("apt") {
        return Ok(AndroidManager::Apt);
    }

    Err(UpkgError::Unsupported(
        "no supported Android package manager found (pkg/apt)",
    ))
}

impl AndroidManager {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Pkg => "pkg",
            Self::Apt => "apt",
        }
    }

    pub fn install_spec(&self, packages: &[String]) -> CommandSpec {
        let mut args: Vec<String> = match self {
            Self::Pkg => vec!["install".into(), "-y".into()],
            Self::Apt => vec!["install".into(), "-y".into()],
        };
        args.extend(packages.iter().cloned());

        match self {
            Self::Pkg => CommandSpec::new("pkg", args),
            Self::Apt => CommandSpec::new("apt", args),
        }
    }

    pub fn uninstall_spec(&self, packages: &[String]) -> CommandSpec {
        let mut args: Vec<String> = match self {
            Self::Pkg => vec!["uninstall".into(), "-y".into()],
            Self::Apt => vec!["remove".into(), "-y".into()],
        };
        args.extend(packages.iter().cloned());

        match self {
            Self::Pkg => CommandSpec::new("pkg", args),
            Self::Apt => CommandSpec::new("apt", args),
        }
    }

    pub fn upgrade_spec(&self, packages: &[String]) -> CommandSpec {
        let mut args: Vec<String> = match self {
            Self::Pkg => vec!["upgrade".into(), "-y".into()],
            Self::Apt => {
                if packages.is_empty() {
                    vec!["upgrade".into(), "-y".into()]
                } else {
                    vec!["install".into(), "--only-upgrade".into(), "-y".into()]
                }
            }
        };
        args.extend(packages.iter().cloned());

        match self {
            Self::Pkg => CommandSpec::new("pkg", args),
            Self::Apt => CommandSpec::new("apt", args),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case(AndroidManager::Pkg, "pkg")]
    #[case(AndroidManager::Apt, "apt")]
    fn manager_name(#[case] manager: AndroidManager, #[case] expected: &str) {
        assert_eq!(manager.name(), expected);
    }

    #[rstest]
    #[case(AndroidManager::Pkg, vec!["git".into()], "pkg", vec!["install", "-y", "git"])]
    #[case(AndroidManager::Apt, vec!["curl".into()], "apt", vec!["install", "-y", "curl"])]
    fn install_spec_generates_correct_commands(
        #[case] manager: AndroidManager,
        #[case] packages: Vec<String>,
        #[case] expected_command: &str,
        #[case] expected_args: Vec<&str>,
    ) {
        let spec = manager.install_spec(&packages);
        assert_eq!(spec.command(), expected_command);

        let args: Vec<&str> = spec.args().iter().map(|s| s.as_str()).collect();
        assert_eq!(args, expected_args);
    }

    #[test]
    fn install_spec_handles_multiple_packages() {
        let manager = AndroidManager::Pkg;
        let packages = vec!["vim".into(), "wget".into(), "git".into()];
        let spec = manager.install_spec(&packages);

        let args: Vec<&str> = spec.args().iter().map(|s| s.as_str()).collect();
        assert_eq!(args, vec!["install", "-y", "vim", "wget", "git"]);
    }

    #[rstest]
    #[case(AndroidManager::Pkg, vec!["git".into()], "pkg", vec!["uninstall", "-y", "git"])]
    #[case(AndroidManager::Apt, vec!["curl".into()], "apt", vec!["remove", "-y", "curl"])]
    fn uninstall_spec_generates_correct_commands(
        #[case] manager: AndroidManager,
        #[case] packages: Vec<String>,
        #[case] expected_command: &str,
        #[case] expected_args: Vec<&str>,
    ) {
        let spec = manager.uninstall_spec(&packages);
        assert_eq!(spec.command(), expected_command);

        let args: Vec<&str> = spec.args().iter().map(|s| s.as_str()).collect();
        assert_eq!(args, expected_args);
    }

    #[test]
    fn upgrade_spec_handles_empty_packages() {
        let manager = AndroidManager::Pkg;
        let packages: Vec<String> = vec![];
        let spec = manager.upgrade_spec(&packages);

        let args: Vec<&str> = spec.args().iter().map(|s| s.as_str()).collect();
        assert_eq!(args, vec!["upgrade", "-y"]);
    }
}
