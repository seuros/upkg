use std::ffi::OsStr;

use super::{Cli, CommandKind, PackageKind};
use crate::error::UpkgError;
use usage_rs as usage;

/// Unified package manager frontend
#[derive(usage::Cli)]
#[usage(
    bin = "upkg",
    version = env!("CARGO_PKG_VERSION"),
    unknown_flags = "error",
    after_help = "Examples:\n  upkg install curl git\n  upkg install --app ghostty\n  upkg upgrade --dry-run neovim\n  upkg search --exact git\n\nCompatibility: --self-upgrade is an alias for self-upgrade."
)]
struct Arguments {
    #[usage(subcommand)]
    command: Commands,
}

#[derive(usage::Subcommands)]
enum Commands {
    /// Install one or more packages
    #[usage(alias = "i")]
    Install(RequiredPackages),
    /// Uninstall one or more packages
    #[usage(alias = "remove", alias = "rm")]
    Uninstall(RequiredPackages),
    /// Upgrade selected packages, or all packages when none are given
    #[usage(alias = "update")]
    Upgrade(OptionalPackages),
    /// List installed packages
    #[usage(alias = "ls")]
    List,
    /// Search for packages
    #[usage(alias = "s")]
    Search(Search),
    /// Upgrade upkg itself
    SelfUpgrade(SelfUpgrade),
    /// Diagnose the local package manager setup
    #[usage(alias = "doctor")]
    Shaman,
}

#[derive(usage::Args)]
struct PackageOptions {
    /// Show the planned operation without executing it
    #[usage(long, var)]
    dry_run: bool,
    /// Select macOS applications (casks)
    #[usage(long, var)]
    app: bool,
}

#[derive(usage::Args)]
#[usage(unknown_flags = "value")]
struct RequiredPackages {
    #[usage(flatten)]
    options: PackageOptions,
    /// Packages to operate on
    #[usage(required = true)]
    packages: Vec<String>,
}

#[derive(usage::Args)]
#[usage(unknown_flags = "value")]
struct OptionalPackages {
    #[usage(flatten)]
    options: PackageOptions,
    /// Packages to upgrade (omit to upgrade all)
    packages: Vec<String>,
}

#[derive(usage::Args)]
#[usage(unknown_flags = "value")]
struct Search {
    /// Search macOS applications (casks)
    #[usage(long, var)]
    app: bool,
    /// Match the package name exactly
    #[usage(long, short = 'e', var)]
    exact: bool,
    /// Refresh the search index
    #[usage(long, var)]
    refresh: bool,
    /// Search terms, joined with spaces
    #[usage(required = true)]
    query: Vec<String>,
}

#[derive(usage::Args)]
struct SelfUpgrade {
    /// Show the planned upgrade without executing it
    #[usage(long, var)]
    dry_run: bool,
}

fn package_kind(app: bool) -> PackageKind {
    if app {
        PackageKind::App
    } else {
        PackageKind::Auto
    }
}

pub(super) fn parse(args: impl Iterator<Item = String>) -> Result<Cli, UpkgError> {
    let mut values: Vec<String> = args.collect();
    // Normalize only the command token, leaving package names and search terms untouched.
    if let Some(first) = values.first_mut()
        && first == "--self-upgrade"
    {
        *first = "self-upgrade".to_owned();
    }
    let argv: Vec<&OsStr> = values.iter().map(OsStr::new).collect();
    let command = match Arguments::parse_from(&argv) {
        Ok(arguments) => match arguments.command {
            Commands::Install(args) => CommandKind::Install {
                packages: args.packages,
                dry_run: args.options.dry_run,
                kind: package_kind(args.options.app),
            },
            Commands::Uninstall(args) => CommandKind::Uninstall {
                packages: args.packages,
                dry_run: args.options.dry_run,
                kind: package_kind(args.options.app),
            },
            Commands::Upgrade(args) => CommandKind::Upgrade {
                packages: args.packages,
                dry_run: args.options.dry_run,
                kind: package_kind(args.options.app),
            },
            Commands::List => CommandKind::List,
            Commands::Search(args) => CommandKind::Search {
                query: args.query.join(" "),
                exact: args.exact,
                kind: package_kind(args.app),
                refresh: args.refresh,
            },
            Commands::SelfUpgrade(args) => CommandKind::SelfUpgrade {
                dry_run: args.dry_run,
            },
            Commands::Shaman => CommandKind::Shaman,
        },
        Err(usage::Error::Help { cmd, long }) => CommandKind::Help(
            usage::help::render(Arguments::spec(), cmd, long)
                .expect("help command belongs to the CLI spec"),
        ),
        Err(usage::Error::Version { .. }) => CommandKind::Version,
        Err(error) => {
            return Err(UpkgError::Usage(usage::render_failure_plain(
                Arguments::spec(),
                &argv,
                &error,
            )));
        }
    };
    Ok(Cli { command })
}
