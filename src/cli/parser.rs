use std::ffi::OsString;
use std::path::PathBuf;

use super::{Cli, CommandKind, ManifestSource, PackageAction, PackageKind, usage_error};
use crate::error::UpkgError;
use upkg::host::Task;
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
    example = "upkg install --dry-run",
    example = "upkg install --app ghostty",
    example = "upkg upgrade --dry-run neovim",
    example = "upkg search --exact git",
    example = "upkg refresh && upkg upgrade --full && upkg cleanup",
    example = "upkg reboot-required --json",
    after_help = "Compatibility: --self-upgrade is an alias for self-upgrade."
)]
struct Arguments {
    #[usage(subcommand)]
    command: Commands,
}

#[derive(usage::Subcommands)]
enum Commands {
    /// Install packages, or what ./upkg.toml lists when none are named
    #[usage(alias = "i", example = "upkg install --manifest ci/upkg.toml")]
    Install(InstallPackages),
    /// Uninstall one or more packages
    #[usage(alias = "remove", alias = "rm")]
    Uninstall(RequiredPackages),
    /// Upgrade selected packages, or all packages when none are given
    #[usage(alias = "update", example = "upkg upgrade --full --dry-run")]
    Upgrade(OptionalPackages),
    /// Reinstall installed packages, keeping the old copy if it fails
    Reinstall(RequiredPackages),
    /// List installed packages
    #[usage(alias = "ls")]
    List,
    /// Check whether packages (or what ./upkg.toml lists) are installed; exits 1 when any is missing
    #[usage(example = "upkg check git libpq-dev", example = "upkg check")]
    Check(CheckPackages),
    /// Search for packages
    #[usage(alias = "s")]
    Search(Search),
    /// Refresh package metadata without upgrading anything
    Refresh(Maintenance),
    /// Remove orphaned packages and clear the package cache
    Cleanup(Maintenance),
    /// Tell whether the host must reboot to run what is installed
    #[usage(example = "upkg reboot-required --json")]
    RebootRequired(RebootRequired),
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
struct InstallPackages {
    #[usage(flatten)]
    options: PackageOptions,
    /// Read the packages from this upkg.toml instead of ./upkg.toml
    #[usage(long, value_name = "PATH")]
    manifest: Option<PathBuf>,
    /// Packages to install (omit to install what upkg.toml lists)
    packages: Vec<String>,
}

#[derive(usage::Args)]
#[usage(unknown_flags = "value")]
struct OptionalPackages {
    #[usage(flatten)]
    options: PackageOptions,
    /// Upgrade the whole system, new kernels and the base system included; takes no packages
    #[usage(long, var)]
    full: bool,
    /// Packages to upgrade (omit to upgrade all)
    packages: Vec<String>,
}

#[derive(usage::Args)]
struct Maintenance {
    /// Show the planned commands without running them
    #[usage(long, var)]
    dry_run: bool,
}

#[derive(usage::Args)]
struct RebootRequired {
    /// Print the answer as one JSON object
    #[usage(long, var)]
    json: bool,
}

#[derive(usage::Args)]
#[usage(unknown_flags = "value")]
struct CheckPackages {
    /// Read the packages from this upkg.toml instead of ./upkg.toml
    #[usage(long, value_name = "PATH")]
    manifest: Option<PathBuf>,
    /// Packages to look for (omit to check what upkg.toml lists)
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

/// `upgrade --full` is the whole system or nothing: no names, no casks.
fn upgrade(args: OptionalPackages) -> Result<CommandKind, UpkgError> {
    if !args.full {
        return Ok(package(PackageAction::Upgrade, args.options, args.packages));
    }
    let conflict = if !args.packages.is_empty() {
        Some("upgrade --full upgrades everything and takes no package names")
    } else if args.options.app {
        Some("upgrade --full cannot be combined with --app")
    } else {
        None
    };
    if let Some(conflict) = conflict {
        return Err(usage_error("upgrade", conflict));
    }
    Ok(CommandKind::Maintain {
        task: Task::FullUpgrade,
        dry_run: args.options.dry_run,
    })
}

fn package(action: PackageAction, options: PackageOptions, packages: Vec<String>) -> CommandKind {
    CommandKind::Package {
        action,
        packages,
        manifest: None,
        dry_run: options.dry_run,
        kind: package_kind(options.app),
    }
}

/// Named packages, or else a manifest: `--manifest PATH`, or ./upkg.toml.
fn manifest_source(
    command: &str,
    manifest: Option<PathBuf>,
    packages: &[String],
) -> Result<Option<ManifestSource>, UpkgError> {
    match (manifest, packages.is_empty()) {
        (Some(_), false) => Err(usage_error(
            command,
            "name packages or pass --manifest, not both",
        )),
        (Some(path), true) => Ok(Some(ManifestSource::Path(path))),
        (None, true) => Ok(Some(ManifestSource::WorkingDir)),
        (None, false) => Ok(None),
    }
}

fn install(args: InstallPackages) -> Result<CommandKind, UpkgError> {
    let manifest = manifest_source("install", args.manifest, &args.packages)?;
    if manifest.is_some() && args.options.app {
        return Err(usage_error(
            "install",
            "--app needs app names; upkg.toml lists system packages",
        ));
    }
    Ok(CommandKind::Package {
        action: PackageAction::Install,
        packages: args.packages,
        manifest,
        dry_run: args.options.dry_run,
        kind: package_kind(args.options.app),
    })
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
        Commands::Install(args) => install(args)?,
        Commands::Uninstall(args) => package(PackageAction::Uninstall, args.options, args.packages),
        Commands::Upgrade(args) => upgrade(args)?,
        Commands::Reinstall(args) => package(PackageAction::Reinstall, args.options, args.packages),
        Commands::List => CommandKind::List,
        Commands::Check(args) => CommandKind::Check {
            manifest: manifest_source("check", args.manifest, &args.packages)?,
            packages: args.packages,
        },
        Commands::Search(args) => CommandKind::Search {
            query: args.query.join(" "),
            exact: args.exact,
            kind: package_kind(args.app),
            refresh: args.refresh,
        },
        Commands::Refresh(args) => CommandKind::Maintain {
            task: Task::Refresh,
            dry_run: args.dry_run,
        },
        Commands::Cleanup(args) => CommandKind::Maintain {
            task: Task::Cleanup,
            dry_run: args.dry_run,
        },
        Commands::RebootRequired(args) => CommandKind::RebootRequired { json: args.json },
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
