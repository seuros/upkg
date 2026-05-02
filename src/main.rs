#[cfg(target_os = "macos")]
mod api;
mod backend;
mod cli;
#[cfg(target_os = "macos")]
mod core;
mod error;
#[cfg(target_os = "macos")]
mod init;
#[cfg(target_os = "macos")]
mod native;
#[cfg(target_os = "macos")]
mod native_cli;
#[cfg(target_os = "macos")]
mod types;

#[cfg(target_os = "macos")]
pub use types::*;

use std::process::ExitCode;

use backend::Backend;
use cli::{Cli, CommandKind, PackageKind};
use error::UpkgError;

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<ExitCode, UpkgError> {
    let cli = Cli::parse(std::env::args().skip(1))?;

    match cli.command {
        CommandKind::Install {
            packages,
            dry_run,
            kind,
        } => install(&packages, dry_run, kind),
        CommandKind::Uninstall {
            packages,
            dry_run,
            kind,
        } => uninstall(&packages, dry_run, kind),
        CommandKind::Upgrade {
            packages,
            dry_run,
            kind,
        } => upgrade(&packages, dry_run, kind),
        CommandKind::Help => {
            println!("{}", Cli::help_text());
            Ok(ExitCode::SUCCESS)
        }
        CommandKind::Version => {
            println!("upkg {}", env!("CARGO_PKG_VERSION"));
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn install(packages: &[String], dry_run: bool, kind: PackageKind) -> Result<ExitCode, UpkgError> {
    let backend = Backend::detect()?;
    let spec = backend.install_spec(packages);

    if dry_run {
        return print_dry_run(&backend, &spec);
    }

    #[cfg(target_os = "macos")]
    {
        let _ = (backend, spec);
        native::install_native(packages, kind)?;
        Ok(ExitCode::SUCCESS)
    }

    #[cfg(not(target_os = "macos"))]
    {
        execute_spec(backend, spec)
    }
}

fn uninstall(packages: &[String], dry_run: bool, kind: PackageKind) -> Result<ExitCode, UpkgError> {
    let backend = Backend::detect()?;
    let spec = backend.uninstall_spec(packages);

    if dry_run {
        return print_dry_run(&backend, &spec);
    }

    #[cfg(target_os = "macos")]
    {
        let _ = (backend, spec);
        native::uninstall_native(packages, kind)?;
        Ok(ExitCode::SUCCESS)
    }

    #[cfg(not(target_os = "macos"))]
    {
        execute_spec(backend, spec)
    }
}

fn upgrade(packages: &[String], dry_run: bool, kind: PackageKind) -> Result<ExitCode, UpkgError> {
    let backend = Backend::detect()?;
    let spec = backend.upgrade_spec(packages);

    if dry_run {
        return print_dry_run(&backend, &spec);
    }

    #[cfg(target_os = "macos")]
    {
        let _ = (backend, spec);
        native::upgrade_native(packages, kind)?;
        Ok(ExitCode::SUCCESS)
    }

    #[cfg(not(target_os = "macos"))]
    {
        execute_spec(backend, spec)
    }
}

fn print_dry_run(backend: &Backend, spec: &backend::CommandSpec) -> Result<ExitCode, UpkgError> {
    println!("backend: {}", backend.name());
    println!("dry-run: {}", spec.render());
    Ok(ExitCode::SUCCESS)
}

#[cfg(not(target_os = "macos"))]
fn execute_spec(backend: Backend, spec: backend::CommandSpec) -> Result<ExitCode, UpkgError> {
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
