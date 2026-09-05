use crate::error::UpkgError;

mod parser;

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
    Help(String),
    Version,
    SelfUpgrade {
        dry_run: bool,
    },
    Shaman,
}

impl Cli {
    pub fn parse(args: impl Iterator<Item = String>) -> Result<Self, UpkgError> {
        parser::parse(args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case(&["i", "git"], "install")]
    #[case(&["remove", "git"], "uninstall")]
    #[case(&["rm", "git"], "uninstall")]
    #[case(&["update"], "upgrade")]
    #[case(&["ls"], "list")]
    #[case(&["doctor"], "shaman")]
    #[case(&["--self-upgrade", "--dry-run"], "self-upgrade")]
    fn aliases(#[case] args: &[&str], #[case] expected: &str) {
        let cli = Cli::parse(args.iter().map(|arg| (*arg).to_owned())).unwrap();
        let name = match cli.command {
            CommandKind::Install { .. } => "install",
            CommandKind::Uninstall { .. } => "uninstall",
            CommandKind::Upgrade { .. } => "upgrade",
            CommandKind::List => "list",
            CommandKind::Shaman => "shaman",
            CommandKind::SelfUpgrade { dry_run: true } => "self-upgrade",
            _ => panic!("unexpected command"),
        };
        assert_eq!(name, expected);
    }

    #[rstest]
    #[case(&[], "command")]
    #[case(&["nonesuch"], "nonesuch")]
    #[case(&["--unknown"], "--unknown")]
    #[case(&["install", "--app"], "<PACKAGES>")]
    #[case(&["uninstall", "--dry-run"], "<PACKAGES>")]
    #[case(&["search", "--exact"], "<QUERY>")]
    #[case(&["list", "extra"], "extra")]
    #[case(&["shaman", "extra"], "extra")]
    #[case(&["self-upgrade", "--app"], "--app")]
    #[case(&["--self-upgrade", "extra"], "extra")]
    fn invalid_arguments(#[case] args: &[&str], #[case] expected: &str) {
        let error = Cli::parse(args.iter().map(|arg| (*arg).to_owned())).unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
    }

    #[rstest]
    #[case(&["help"], "install")]
    #[case(&["--help"], "install")]
    #[case(&["-h"], "install")]
    #[case(&["install", "--help"], "--dry-run")]
    #[case(&["help", "install"], "--dry-run")]
    #[case(&["i", "-h"], "--app")]
    #[case(&["search", "--help"], "--refresh")]
    #[case(&["--self-upgrade", "--help"], "--dry-run")]
    fn generated_help(#[case] args: &[&str], #[case] expected: &str) {
        let cli = Cli::parse(args.iter().map(|arg| (*arg).to_owned())).unwrap();
        let CommandKind::Help(text) = cli.command else {
            panic!("expected help");
        };
        assert!(text.contains(expected), "{text}");
        assert!(text.contains("upkg"), "{text}");
    }

    #[test]
    fn short_version_flag() {
        let cli = Cli::parse(["-V".to_owned()].into_iter()).unwrap();
        assert!(matches!(cli.command, CommandKind::Version));
    }

    #[test]
    fn package_options_can_follow_values_and_repeat() {
        let cli = Cli::parse(
            [
                "install",
                "git",
                "--dry-run",
                "curl",
                "--app",
                "--dry-run",
                "--app",
            ]
            .into_iter()
            .map(str::to_owned),
        )
        .unwrap();
        let CommandKind::Install {
            packages,
            dry_run,
            kind,
        } = cli.command
        else {
            panic!("expected install");
        };
        assert_eq!(packages, ["git", "curl"]);
        assert!(dry_run);
        assert_eq!(kind, PackageKind::App);
    }

    #[rstest]
    #[case("install")]
    #[case("uninstall")]
    #[case("upgrade")]
    fn package_double_dash(#[case] command: &str) {
        let cli = Cli::parse(
            [command, "--", "--app", "--help"]
                .into_iter()
                .map(str::to_owned),
        )
        .unwrap();
        let (CommandKind::Install { packages, kind, .. }
        | CommandKind::Uninstall { packages, kind, .. }
        | CommandKind::Upgrade { packages, kind, .. }) = cli.command
        else {
            panic!("expected package command");
        };
        assert_eq!(packages, ["--app", "--help"]);
        assert_eq!(kind, PackageKind::Auto);
    }

    #[rstest]
    #[case("install")]
    #[case("uninstall")]
    #[case("upgrade")]
    #[case("search")]
    fn unknown_flags_remain_positional_values(#[case] command: &str) {
        let cli = Cli::parse([command, "--unknown"].into_iter().map(str::to_owned)).unwrap();
        match cli.command {
            CommandKind::Install { packages, .. }
            | CommandKind::Uninstall { packages, .. }
            | CommandKind::Upgrade { packages, .. } => assert_eq!(packages, ["--unknown"]),
            CommandKind::Search { query, .. } => assert_eq!(query, "--unknown"),
            _ => panic!("unexpected command"),
        }
    }

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
        assert!(err.to_string().contains("<PACKAGES>"));
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
        assert!(err.to_string().contains("<PACKAGES>"));
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
        assert!(err.to_string().contains("--verbose"));
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
        assert!(err.to_string().contains("<QUERY>"));
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
