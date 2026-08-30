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
    List,
    Search {
        query: String,
        exact: bool,
        kind: PackageKind,
        refresh: bool,
    },
    Help,
    Version,
    SelfUpgrade {
        dry_run: bool,
    },
    Shaman,
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
            "list" | "ls" => Self::parse_list(values),
            "search" | "s" => Self::parse_search(values),
            "shaman" | "doctor" => Self::parse_shaman(values),
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

    fn parse_shaman(values: Vec<String>) -> Result<Self, UpkgError> {
        if values.len() != 1 {
            return Err(UpkgError::Usage("shaman does not accept arguments yet"));
        }

        Ok(Self {
            command: CommandKind::Shaman,
        })
    }

    fn parse_list(values: Vec<String>) -> Result<Self, UpkgError> {
        if values.len() != 1 {
            return Err(UpkgError::Usage("list does not accept arguments yet"));
        }

        Ok(Self {
            command: CommandKind::List,
        })
    }

    fn parse_search(values: Vec<String>) -> Result<Self, UpkgError> {
        let mut exact = false;
        let mut kind = PackageKind::Auto;
        let mut refresh = false;
        let mut query_tokens: Vec<String> = Vec::new();
        let mut positional_only = false;

        for arg in values.into_iter().skip(1) {
            if positional_only {
                query_tokens.push(arg);
                continue;
            }
            match arg.as_str() {
                "--" => positional_only = true,
                "--exact" | "-e" => exact = true,
                "--app" => kind = PackageKind::App,
                "--refresh" => refresh = true,
                _ => query_tokens.push(arg),
            }
        }

        if query_tokens.is_empty() {
            return Err(UpkgError::Usage("search requires a query"));
        }

        let query = query_tokens.join(" ");

        Ok(Self {
            command: CommandKind::Search {
                query,
                exact,
                kind,
                refresh,
            },
        })
    }

    pub fn help_text() -> &'static str {
        concat!(
            "upkg v",
            env!("CARGO_PKG_VERSION"),
            " - unified package manager frontend\n\nUSAGE:\n  upkg install [--app] [--dry-run] <package> [package...]\n  upkg uninstall [--app] [--dry-run] <package> [package...]\n  upkg upgrade [--app] [--dry-run] [package...]\n  upkg list\n  upkg search [--app] [--exact] [--refresh] <query...>\n  upkg shaman\n  upkg doctor\n  upkg --self-upgrade [--dry-run]\n  upkg --version\n\nEXAMPLES:\n  upkg install curl git\n  upkg install --app ghostty\n  upkg uninstall jq\n  upkg upgrade\n  upkg upgrade --dry-run neovim\n  upkg list\n  upkg search ripgrep\n  upkg search --app ghostty\n  upkg search --exact git\n  upkg shaman\n  upkg doctor\n  upkg --self-upgrade\n"
        )
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
    fn parse_list() {
        let cli =
            Cli::parse(["list"].into_iter().map(str::to_string)).expect("parse should succeed");

        assert!(matches!(cli.command, CommandKind::List));
    }

    #[test]
    fn parse_shaman_and_doctor_alias() {
        let doctor =
            Cli::parse(["doctor"].into_iter().map(str::to_string)).expect("parse should succeed");
        assert!(matches!(doctor.command, CommandKind::Shaman));

        let shaman =
            Cli::parse(["shaman"].into_iter().map(str::to_string)).expect("parse should succeed");
        assert!(matches!(shaman.command, CommandKind::Shaman));
    }

    #[test]
    fn parse_shaman_rejects_arguments() {
        let err = Cli::parse(["shaman", "--verbose"].into_iter().map(str::to_string))
            .expect_err("shaman should reject arguments");
        assert!(err.to_string().contains("shaman does not accept arguments"));
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
    fn parse_search_simple() {
        let cli =
            Cli::parse(["search", "ripgrep"].into_iter().map(str::to_string)).expect("parse ok");

        match cli.command {
            CommandKind::Search {
                query,
                exact,
                kind,
                refresh,
            } => {
                assert_eq!(query, "ripgrep");
                assert!(!exact);
                assert!(!refresh);
                assert_eq!(kind, PackageKind::Auto);
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parse_search_joins_multi_word_query() {
        let cli = Cli::parse(
            ["search", "visual", "studio", "code"]
                .into_iter()
                .map(str::to_string),
        )
        .expect("parse ok");

        match cli.command {
            CommandKind::Search { query, .. } => assert_eq!(query, "visual studio code"),
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parse_search_alias_and_flags() {
        let cli = Cli::parse(
            ["s", "--app", "--exact", "--refresh", "ghostty"]
                .into_iter()
                .map(str::to_string),
        )
        .expect("parse ok");

        match cli.command {
            CommandKind::Search {
                query,
                exact,
                kind,
                refresh,
            } => {
                assert_eq!(query, "ghostty");
                assert!(exact);
                assert!(refresh);
                assert_eq!(kind, PackageKind::App);
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parse_search_double_dash_passes_through_flag_like_queries() {
        let cli = Cli::parse(["search", "--", "--exact"].into_iter().map(str::to_string))
            .expect("parse ok");

        match cli.command {
            CommandKind::Search { query, exact, .. } => {
                assert_eq!(query, "--exact");
                assert!(!exact);
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parse_search_requires_query() {
        let err = Cli::parse(["search"].into_iter().map(str::to_string))
            .expect_err("search needs a query");
        assert!(err.to_string().contains("search requires a query"));
    }

    #[test]
    fn parse_search_short_exact_flag() {
        let cli =
            Cli::parse(["search", "-e", "git"].into_iter().map(str::to_string)).expect("parse ok");

        match cli.command {
            CommandKind::Search { exact, .. } => assert!(exact),
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
