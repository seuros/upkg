use crate::error::UpkgError;

#[derive(Debug)]
pub struct Cli {
    pub command: CommandKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageKind {
    Auto,
    App,
}

#[derive(Debug)]
pub enum CommandKind {
    Install {
        packages: Vec<String>,
        dry_run: bool,
        kind: PackageKind,
    },
    Uninstall {
        packages: Vec<String>,
        dry_run: bool,
        kind: PackageKind,
    },
    Upgrade {
        packages: Vec<String>,
        dry_run: bool,
        kind: PackageKind,
    },
    Help,
    Version,
    SelfUpgrade {
        dry_run: bool,
    },
}

impl Cli {
    pub fn parse(args: impl Iterator<Item = String>) -> Result<Self, UpkgError> {
        let values: Vec<String> = args.collect();
        if values.is_empty() {
            return Err(UpkgError::Usage("missing command"));
        }

        match values[0].as_str() {
            "install" | "i" => Self::parse_install(values),
            "uninstall" | "remove" | "rm" => Self::parse_uninstall(values),
            "upgrade" | "update" => Self::parse_upgrade(values),
            "--self-upgrade" | "self-upgrade" => Self::parse_self_upgrade(values),
            "help" | "--help" | "-h" => Ok(Self {
                command: CommandKind::Help,
            }),
            "--version" | "-V" => Ok(Self {
                command: CommandKind::Version,
            }),
            _ => Err(UpkgError::Usage("unsupported command")),
        }
    }

    fn parse_install(values: Vec<String>) -> Result<Self, UpkgError> {
        let mut dry_run = false;
        let mut kind = PackageKind::Auto;
        let mut packages = Vec::new();

        for arg in values.into_iter().skip(1) {
            match arg.as_str() {
                "--dry-run" => dry_run = true,
                "--app" => kind = PackageKind::App,
                _ => packages.push(arg),
            }
        }

        if packages.is_empty() {
            return Err(UpkgError::Usage("install requires at least one package"));
        }

        Ok(Self {
            command: CommandKind::Install {
                packages,
                dry_run,
                kind,
            },
        })
    }

    fn parse_uninstall(values: Vec<String>) -> Result<Self, UpkgError> {
        let mut dry_run = false;
        let mut kind = PackageKind::Auto;
        let mut packages = Vec::new();

        for arg in values.into_iter().skip(1) {
            match arg.as_str() {
                "--dry-run" => dry_run = true,
                "--app" => kind = PackageKind::App,
                _ => packages.push(arg),
            }
        }

        if packages.is_empty() {
            return Err(UpkgError::Usage("uninstall requires at least one package"));
        }

        Ok(Self {
            command: CommandKind::Uninstall {
                packages,
                dry_run,
                kind,
            },
        })
    }

    fn parse_upgrade(values: Vec<String>) -> Result<Self, UpkgError> {
        let mut dry_run = false;
        let mut kind = PackageKind::Auto;
        let mut packages = Vec::new();

        for arg in values.into_iter().skip(1) {
            match arg.as_str() {
                "--dry-run" => dry_run = true,
                "--app" => kind = PackageKind::App,
                _ => packages.push(arg),
            }
        }

        Ok(Self {
            command: CommandKind::Upgrade {
                packages,
                dry_run,
                kind,
            },
        })
    }

    fn parse_self_upgrade(values: Vec<String>) -> Result<Self, UpkgError> {
        let mut dry_run = false;

        for arg in values.into_iter().skip(1) {
            match arg.as_str() {
                "--dry-run" => dry_run = true,
                _ => return Err(UpkgError::Usage("self-upgrade only accepts --dry-run")),
            }
        }

        Ok(Self {
            command: CommandKind::SelfUpgrade { dry_run },
        })
    }

    pub fn help_text() -> &'static str {
        "upkg - unified package manager frontend\n\nUSAGE:\n  upkg install [--app] [--dry-run] <package> [package...]\n  upkg uninstall [--app] [--dry-run] <package> [package...]\n  upkg upgrade [--app] [--dry-run] [package...]\n  upkg --self-upgrade [--dry-run]\n  upkg --version\n\nEXAMPLES:\n  upkg install curl git\n  upkg install --app ghostty\n  upkg uninstall jq\n  upkg upgrade\n  upkg upgrade --dry-run neovim\n  upkg --self-upgrade\n"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_install_with_dry_run() {
        let cli = Cli::parse(
            ["install", "--dry-run", "git"]
                .into_iter()
                .map(str::to_string),
        )
        .expect("parse should succeed");

        match cli.command {
            CommandKind::Install {
                packages,
                dry_run,
                kind,
            } => {
                assert_eq!(packages, vec!["git"]);
                assert!(dry_run);
                assert_eq!(kind, PackageKind::Auto);
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parse_install_requires_package() {
        let err =
            Cli::parse(["install"].into_iter().map(str::to_string)).expect_err("parse should fail");
        assert!(
            err.to_string()
                .contains("install requires at least one package")
        );
    }

    #[test]
    fn parse_uninstall_with_dry_run() {
        let cli = Cli::parse(
            ["uninstall", "--dry-run", "git"]
                .into_iter()
                .map(str::to_string),
        )
        .expect("parse should succeed");

        match cli.command {
            CommandKind::Uninstall {
                packages,
                dry_run,
                kind,
            } => {
                assert_eq!(packages, vec!["git"]);
                assert!(dry_run);
                assert_eq!(kind, PackageKind::Auto);
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parse_uninstall_requires_package() {
        let err = Cli::parse(["uninstall"].into_iter().map(str::to_string))
            .expect_err("parse should fail");
        assert!(
            err.to_string()
                .contains("uninstall requires at least one package")
        );
    }

    #[test]
    fn parse_upgrade_allows_no_package() {
        let cli =
            Cli::parse(["upgrade"].into_iter().map(str::to_string)).expect("parse should succeed");

        match cli.command {
            CommandKind::Upgrade {
                packages,
                dry_run,
                kind,
            } => {
                assert!(packages.is_empty());
                assert!(!dry_run);
                assert_eq!(kind, PackageKind::Auto);
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parse_install_app_hint() {
        let cli = Cli::parse(
            ["install", "--app", "ghostty"]
                .into_iter()
                .map(str::to_string),
        )
        .expect("parse should succeed");

        match cli.command {
            CommandKind::Install {
                packages,
                dry_run,
                kind,
            } => {
                assert_eq!(packages, vec!["ghostty"]);
                assert!(!dry_run);
                assert_eq!(kind, PackageKind::App);
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parse_version_flag() {
        let cli = Cli::parse(["--version"].into_iter().map(str::to_string))
            .expect("parse should succeed");

        match cli.command {
            CommandKind::Version => {}
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parse_self_upgrade_flag() {
        let cli = Cli::parse(["--self-upgrade"].into_iter().map(str::to_string))
            .expect("parse should succeed");

        match cli.command {
            CommandKind::SelfUpgrade { dry_run } => assert!(!dry_run),
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parse_self_upgrade_dry_run() {
        let cli = Cli::parse(
            ["self-upgrade", "--dry-run"]
                .into_iter()
                .map(str::to_string),
        )
        .expect("parse should succeed");

        match cli.command {
            CommandKind::SelfUpgrade { dry_run } => assert!(dry_run),
            _ => panic!("unexpected command"),
        }
    }
}
