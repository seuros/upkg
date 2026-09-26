use std::ffi::OsString;

use super::{Cli, CommandKind, PackageKind};
use crate::error::UpkgError;
use usage::embedded::Outcome;
use usage_rs as usage;

/// Unified package manager frontend
#[derive(usage::Cli)]
#[usage(
    bin = "upkg",
    version = env!("CARGO_PKG_VERSION"),
    unknown_flags = "error",
    completion,
    spec_endpoint = false,
    example = "upkg install curl git",
    example = "upkg install --app ghostty",
    example = "upkg upgrade --dry-run neovim",
    example = "upkg search --exact git",
    after_help = "Compatibility: --self-upgrade is an alias for self-upgrade."
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
    /// Reinstall installed packages, keeping the old copy if it fails
    Reinstall(RequiredPackages),
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
    /// Print a shell completion script
    #[usage(
        example = "upkg completion zsh > \"${fpath[1]}/_upkg\"",
        example = "upkg completion fish > ~/.config/fish/completions/upkg.fish"
    )]
    Completion(Completion),
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

#[derive(usage::Args)]
struct Completion {
    /// Shell to generate the script for
    #[usage(value_enum)]
    shell: CompletionShell,
}

#[derive(usage::ValueEnum)]
enum CompletionShell {
    Bash,
    Zsh,
    Fish,
    #[usage(name = "powershell", alias = "pwsh")]
    PowerShell,
    Elvish,
    #[usage(alias = "nushell")]
    Nu,
}

impl From<CompletionShell> for usage::complete::Shell {
    fn from(shell: CompletionShell) -> Self {
        match shell {
            CompletionShell::Bash => Self::Bash,
            CompletionShell::Zsh => Self::Zsh,
            CompletionShell::Fish => Self::Fish,
            CompletionShell::PowerShell => Self::PowerShell,
            CompletionShell::Elvish => Self::Elvish,
            CompletionShell::Nu => Self::Nu,
        }
    }
}

fn package_kind(app: bool) -> PackageKind {
    if app {
        PackageKind::App
    } else {
        PackageKind::Auto
    }
}

pub(super) fn parse(args: impl Iterator<Item = OsString>) -> Result<Cli, UpkgError> {
    let mut argv: Vec<OsString> = args.collect();
    // Normalize only the command token, leaving package names and search terms untouched.
    if let Some(first) = argv.first_mut()
        && first == "--self-upgrade"
    {
        *first = "self-upgrade".into();
    }
    let arguments = match Arguments::embedded_outcome(&argv) {
        Outcome::Parsed(arguments) => arguments,
        Outcome::Exit(exit) if exit.stderr => {
            return Err(UpkgError::Usage {
                message: exit.text,
                code: u8::try_from(exit.code).unwrap_or(2),
            });
        }
        Outcome::Exit(exit) => {
            return Ok(Cli {
                command: CommandKind::Print(exit.text),
            });
        }
    };
    let command = match arguments.command {
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
        Commands::Reinstall(args) => CommandKind::Reinstall {
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
        Commands::Completion(args) => {
            CommandKind::Print(Arguments::completion_script(args.shell.into()))
        }
    };
    Ok(Cli { command })
}
