use crate::backend::{CommandSpec, command_exists};
use crate::error::UpkgError;

pub enum WindowsManager {
    Winget,
    Choco,
}

pub fn detect() -> Result<WindowsManager, UpkgError> {
    if command_exists("winget") {
        return Ok(WindowsManager::Winget);
    }
    if command_exists("choco") {
        return Ok(WindowsManager::Choco);
    }

    Err(UpkgError::Unsupported(
        "no supported package manager found (winget/choco)",
    ))
}

impl WindowsManager {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Winget => "winget",
            Self::Choco => "choco",
        }
    }

    pub fn install_spec(&self, packages: &[String]) -> CommandSpec {
        let mut args: Vec<String> = match self {
            Self::Winget => vec![
                "install".into(),
                "--silent".into(),
                "--accept-source-agreements".into(),
                "--accept-package-agreements".into(),
            ],
            Self::Choco => vec!["install".into(), "-y".into()],
        };
        args.extend(packages.iter().cloned());

        match self {
            Self::Winget => CommandSpec::new("winget", args),
            Self::Choco => CommandSpec::new("choco", args),
        }
    }

    pub fn uninstall_spec(&self, packages: &[String]) -> CommandSpec {
        let mut args: Vec<String> = match self {
            Self::Winget => vec![
                "uninstall".into(),
                "--silent".into(),
                "--accept-source-agreements".into(),
            ],
            Self::Choco => vec!["uninstall".into(), "-y".into()],
        };
        args.extend(packages.iter().cloned());

        match self {
            Self::Winget => CommandSpec::new("winget", args),
            Self::Choco => CommandSpec::new("choco", args),
        }
    }

    pub fn upgrade_spec(&self, packages: &[String]) -> CommandSpec {
        let mut args: Vec<String> = match self {
            Self::Winget => vec![
                "upgrade".into(),
                "--silent".into(),
                "--accept-source-agreements".into(),
                "--accept-package-agreements".into(),
            ],
            Self::Choco => vec!["upgrade".into(), "-y".into()],
        };
        args.extend(packages.iter().cloned());

        match self {
            Self::Winget => CommandSpec::new("winget", args),
            Self::Choco => CommandSpec::new("choco", args),
        }
    }

    pub fn search_spec(&self, query: &str, exact: bool) -> Result<CommandSpec, UpkgError> {
        Ok(match self {
            Self::Winget => {
                let mut args = vec!["search".to_string()];
                if exact {
                    args.push("-e".to_string());
                }
                args.push(query.to_string());
                CommandSpec::new("winget", args)
            }
            Self::Choco => {
                let mut args = vec!["search".to_string(), query.to_string()];
                if exact {
                    args.push("-e".to_string());
                }
                CommandSpec::new("choco", args)
            }
        })
    }

    pub fn list_spec(&self) -> CommandSpec {
        match self {
            Self::Winget => CommandSpec::new("winget", vec!["list".to_string()]),
            Self::Choco => CommandSpec::new(
                "choco",
                vec!["list".to_string(), "--local-only".to_string()],
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case(WindowsManager::Winget, "winget")]
    #[case(WindowsManager::Choco, "choco")]
    fn manager_name(#[case] manager: WindowsManager, #[case] expected: &str) {
        assert_eq!(manager.name(), expected);
    }

    #[rstest]
    #[case(WindowsManager::Winget, vec!["git".into()], "winget", vec!["install", "--silent", "--accept-source-agreements", "--accept-package-agreements", "git"])]
    #[case(WindowsManager::Choco, vec!["curl".into()], "choco", vec!["install", "-y", "curl"])]
    fn install_spec_generates_correct_commands(
        #[case] manager: WindowsManager,
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
        let manager = WindowsManager::Winget;
        let packages = vec!["nodejs".into(), "python".into(), "git".into()];
        let spec = manager.install_spec(&packages);

        let args: Vec<&str> = spec.args().iter().map(|s| s.as_str()).collect();
        assert_eq!(
            args,
            vec![
                "install",
                "--silent",
                "--accept-source-agreements",
                "--accept-package-agreements",
                "nodejs",
                "python",
                "git"
            ]
        );
    }

    #[rstest]
    #[case(WindowsManager::Winget, vec!["git".into()], "winget", vec!["uninstall", "--silent", "--accept-source-agreements", "git"])]
    #[case(WindowsManager::Choco, vec!["curl".into()], "choco", vec!["uninstall", "-y", "curl"])]
    fn uninstall_spec_generates_correct_commands(
        #[case] manager: WindowsManager,
        #[case] packages: Vec<String>,
        #[case] expected_command: &str,
        #[case] expected_args: Vec<&str>,
    ) {
        let spec = manager.uninstall_spec(&packages);
        assert_eq!(spec.command(), expected_command);

        let args: Vec<&str> = spec.args().iter().map(|s| s.as_str()).collect();
        assert_eq!(args, expected_args);
    }

    #[rstest]
    #[case(WindowsManager::Winget, false, "winget", vec!["search", "ripgrep"])]
    #[case(WindowsManager::Winget, true, "winget", vec!["search", "-e", "ripgrep"])]
    #[case(WindowsManager::Choco, false, "choco", vec!["search", "ripgrep"])]
    #[case(WindowsManager::Choco, true, "choco", vec!["search", "ripgrep", "-e"])]
    fn search_spec_generates_correct_commands(
        #[case] manager: WindowsManager,
        #[case] exact: bool,
        #[case] expected_command: &str,
        #[case] expected_args: Vec<&str>,
    ) {
        let spec = manager.search_spec("ripgrep", exact).unwrap();
        assert_eq!(spec.command(), expected_command);
        let args: Vec<&str> = spec.args().iter().map(|s| s.as_str()).collect();
        assert_eq!(args, expected_args);
    }

    #[rstest]
    #[case(WindowsManager::Winget, "winget", vec!["list"])]
    #[case(WindowsManager::Choco, "choco", vec!["list", "--local-only"])]
    fn list_spec_generates_correct_commands(
        #[case] manager: WindowsManager,
        #[case] expected_command: &str,
        #[case] expected_args: Vec<&str>,
    ) {
        let spec = manager.list_spec();
        assert_eq!(spec.command(), expected_command);

        let args: Vec<&str> = spec.args().iter().map(|s| s.as_str()).collect();
        assert_eq!(args, expected_args);
    }
}
