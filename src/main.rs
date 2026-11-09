#[cfg(target_os = "macos")]
mod api;
#[cfg(not(target_os = "macos"))]
mod backend;
mod check;
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[path = "core/checksum.rs"]
mod checksum;
mod cli;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod clock;
#[cfg(target_os = "macos")]
mod core;
mod error;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod http_client;
#[cfg(target_os = "macos")]
mod init;
#[cfg(target_os = "macos")]
mod native;
#[cfg(target_os = "macos")]
mod native_cli;
#[cfg(target_os = "macos")]
mod package_ref;
#[cfg(target_os = "macos")]
mod privilege_macos;
mod self_upgrade;
mod shaman;
#[cfg(target_os = "macos")]
mod types;

#[cfg(target_os = "macos")]
pub use types::*;

use std::path::Path;
use std::process::ExitCode;

#[cfg(not(target_os = "macos"))]
use backend::Backend;
use cli::{Cli, CommandKind, ManifestSource, PackageAction, PackageKind};
use error::UpkgError;
use upkg::host::{Host, Options, Privilege, StepState, SystemRunner, Task};
use upkg::manifest::{self, Manifest};

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(UpkgError::Usage { message, code }) => {
            // usage-rs already supplies the error prefix and help hint.
            eprint!("{message}");
            ExitCode::from(code)
        }
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<ExitCode, UpkgError> {
    let cli = Cli::parse(std::env::args_os().skip(1))?;

    match cli.command {
        CommandKind::Package {
            action,
            manifest: Some(source),
            dry_run,
            kind,
            ..
        } => {
            let packages = manifest_packages("install", &source)?;
            if packages.is_empty() {
                eprintln!("note: {} lists no packages", source_path(&source).display());
                return Ok(ExitCode::SUCCESS);
            }
            package(action, &packages, dry_run, kind)
        }
        CommandKind::Package {
            action,
            packages,
            manifest: None,
            dry_run,
            kind,
        } => package(action, &packages, dry_run, kind),
        CommandKind::List => list(),
        CommandKind::Check { packages, manifest } => match manifest {
            Some(source) => {
                let packages = manifest_packages("check", &source)?;
                if packages.is_empty() {
                    // Nothing to ask, so no backend to find.
                    return Ok(ExitCode::SUCCESS);
                }
                check_packages(&packages)
            }
            None => check_packages(&packages),
        },
        CommandKind::Search {
            query,
            exact,
            kind,
            refresh,
        } => search(&query, exact, kind, refresh),
        CommandKind::Maintain { task, dry_run } => maintain(task, dry_run),
        CommandKind::RebootRequired { json } => reboot_required(json),
        CommandKind::Print(text) => {
            print!("{text}");
            Ok(ExitCode::SUCCESS)
        }
        CommandKind::SelfUpgrade { dry_run } => self_upgrade(dry_run),
        CommandKind::Shaman => shaman(),
    }
}

fn source_path(source: &ManifestSource) -> &Path {
    match source {
        ManifestSource::WorkingDir => Path::new(manifest::FILE_NAME),
        ManifestSource::Path(path) => path,
    }
}

/// The names `source` lists, read before any backend is detected. No
/// ./upkg.toml is a usage error: the command was given nothing to work on.
///
/// `install` runs as root, often in a checkout someone else wrote, so the
/// working directory's upkg.toml is read like any untrusted one: a regular
/// file or nothing, never a symlink to wherever (`/dev/zero` hangs a
/// reader, `/etc/shadow` ends up in a parse error). `--manifest PATH` is a
/// path the operator chose, and is followed like any other argument.
fn manifest_packages(command: &str, source: &ManifestSource) -> Result<Vec<String>, UpkgError> {
    let manifest = match source {
        // The empty path is the working directory, and keeps errors naming
        // a plain `upkg.toml`.
        ManifestSource::WorkingDir => Manifest::load_from_dir("")?.ok_or_else(|| {
            cli::usage_error(
                command,
                "no packages named, and no upkg.toml in the current directory",
            )
        })?,
        ManifestSource::Path(path) => Manifest::load(path)?,
    };
    Ok(manifest.packages().to_vec())
}

fn package(
    action: PackageAction,
    packages: &[String],
    dry_run: bool,
    kind: PackageKind,
) -> Result<ExitCode, UpkgError> {
    #[cfg(target_os = "macos")]
    {
        if dry_run {
            native::print_dry_run(action, packages, kind)?;
        } else {
            native::run(action, packages, kind)?;
        }
        Ok(ExitCode::SUCCESS)
    }

    #[cfg(not(target_os = "macos"))]
    {
        reject_app_kind(kind)?;
        let backend = Backend::detect()?;
        let spec = match action {
            PackageAction::Install => backend.install_spec(packages),
            PackageAction::Uninstall => backend.uninstall_spec(packages),
            PackageAction::Upgrade => backend.upgrade_spec(packages),
            PackageAction::Reinstall => backend.reinstall_spec(packages)?,
        }
        .for_current_user();
        let refresh = match action {
            PackageAction::Uninstall => None,
            _ => backend
                .refresh_spec()
                .map(backend::CommandSpec::for_current_user),
        };
        if dry_run {
            return print_dry_run(&backend, refresh.iter().chain([&spec]));
        }
        if let Some(refresh) = refresh {
            execute_spec(&backend, refresh)?;
        }
        execute_spec(&backend, spec)
    }
}

fn list() -> Result<ExitCode, UpkgError> {
    #[cfg(target_os = "macos")]
    {
        native::list_native()?;
        Ok(ExitCode::SUCCESS)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let backend = Backend::detect()?;
        let spec = backend.list_spec();
        execute_spec(&backend, spec)
    }
}

fn check_packages(packages: &[String]) -> Result<ExitCode, UpkgError> {
    #[cfg(target_os = "macos")]
    let results = native::check_native(packages)?;

    #[cfg(not(target_os = "macos"))]
    let results = check::on_backend(&Backend::detect()?, packages)?;

    Ok(check::report(&results))
}

fn search(
    query: &str,
    exact: bool,
    kind: PackageKind,
    refresh: bool,
) -> Result<ExitCode, UpkgError> {
    #[cfg(target_os = "macos")]
    {
        native::search_native(query, exact, kind, refresh)?;
        Ok(ExitCode::SUCCESS)
    }

    #[cfg(not(target_os = "macos"))]
    {
        if refresh {
            return Err(UpkgError::Unsupported(
                "--refresh is only meaningful with the built-in macOS engine",
            ));
        }
        reject_app_kind(kind)?;
        let backend = Backend::detect()?;
        let spec = backend.search_spec(query, exact)?;
        execute_search_spec(spec)
    }
}

#[cfg(not(target_os = "macos"))]
fn print_dry_run<'a>(
    backend: &Backend,
    specs: impl IntoIterator<Item = &'a backend::CommandSpec>,
) -> Result<ExitCode, UpkgError> {
    println!("backend: {}", backend.name());
    for spec in specs {
        println!("dry-run: {}", spec.render());
    }
    Ok(ExitCode::SUCCESS)
}

/// `refresh`, `upgrade --full` and `cleanup`, through the host library. The
/// native tools' output streams through as they run.
fn maintain(task: Task, dry_run: bool) -> Result<ExitCode, UpkgError> {
    let host = Host::detect_with(SystemRunner::echoing())?;
    let options = Options {
        dry_run,
        privilege: Privilege::Auto,
    };
    let outcome = host.perform(task, &options)?;
    if dry_run {
        println!("backend: {}", outcome.manager);
    }
    for step in &outcome.steps {
        let command = step.command.as_deref().unwrap_or_default();
        match &step.state {
            StepState::Planned => println!("dry-run: {command}"),
            StepState::Note { text } => eprintln!("note: {text}"),
            StepState::Skipped { reason } => eprintln!("skipped: {command} ({reason})"),
            StepState::Succeeded | StepState::Settled | StepState::Failed => {}
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Exits 0 whenever the check succeeds, whatever the answer.
fn reboot_required(json: bool) -> Result<ExitCode, UpkgError> {
    let status = Host::detect()?.reboot_required()?;
    if json {
        let line = serde_json::to_string(&status).map_err(std::io::Error::other)?;
        println!("{line}");
    } else {
        println!("{}", status.summary());
    }
    Ok(ExitCode::SUCCESS)
}

fn self_upgrade(dry_run: bool) -> Result<ExitCode, UpkgError> {
    self_upgrade::run(dry_run)?;
    Ok(ExitCode::SUCCESS)
}

fn shaman() -> Result<ExitCode, UpkgError> {
    if shaman::run() {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(1))
    }
}

#[cfg(not(target_os = "macos"))]
fn reject_app_kind(kind: PackageKind) -> Result<(), UpkgError> {
    if kind == PackageKind::App {
        return Err(UpkgError::Unsupported(
            "--app is only available with the built-in macOS engine",
        ));
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn execute_spec(backend: &Backend, spec: backend::CommandSpec) -> Result<ExitCode, UpkgError> {
    let status = spec.into_command().status()?;
    if status.success() {
        Ok(ExitCode::SUCCESS)
    } else {
        Err(UpkgError::CommandFailed {
            command: backend.name().to_string(),
            code: status.code().unwrap_or(1),
        })
    }
}

#[cfg(not(target_os = "macos"))]
fn execute_search_spec(spec: backend::CommandSpec) -> Result<ExitCode, UpkgError> {
    let status = spec.into_command().status()?;
    let code = status.code().unwrap_or(1);
    if status.success() {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(code as u8))
    }
}
