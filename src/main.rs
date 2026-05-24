#[cfg(target_os = "macos")]
mod api;
#[cfg(not(target_os = "macos"))]
mod backend;
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[path = "core/checksum.rs"]
mod checksum;
mod cli;
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
#[cfg(target_os = "macos")]
mod types;

#[cfg(target_os = "macos")]
pub use types::*;

use std::process::ExitCode;

#[cfg(not(target_os = "macos"))]
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
        CommandKind::List => list(),
        CommandKind::Help => {
            println!("{}", Cli::help_text());
            Ok(ExitCode::SUCCESS)
        }
        CommandKind::Version => {
            println!("upkg {}", env!("CARGO_PKG_VERSION"));
            Ok(ExitCode::SUCCESS)
        }
        CommandKind::SelfUpgrade { dry_run } => self_upgrade(dry_run),
    }
}

fn install(packages: &[String], dry_run: bool, kind: PackageKind) -> Result<ExitCode, UpkgError> {
    #[cfg(target_os = "macos")]
    {
        if dry_run {
            native::print_install_dry_run(packages, kind)?;
            return Ok(ExitCode::SUCCESS);
        }

        native::install_native(packages, kind)?;
        Ok(ExitCode::SUCCESS)
    }

    #[cfg(not(target_os = "macos"))]
    {
        reject_app_kind(kind)?;
        let backend = Backend::detect()?;
        let spec = backend.install_spec(packages);
        if dry_run {
            return print_dry_run(&backend, &spec);
        }
        execute_spec(backend, spec)
    }
}

fn uninstall(packages: &[String], dry_run: bool, kind: PackageKind) -> Result<ExitCode, UpkgError> {
    #[cfg(target_os = "macos")]
    {
        if dry_run {
            native::print_uninstall_dry_run(packages, kind)?;
            return Ok(ExitCode::SUCCESS);
        }

        native::uninstall_native(packages, kind)?;
        Ok(ExitCode::SUCCESS)
    }

    #[cfg(not(target_os = "macos"))]
    {
        reject_app_kind(kind)?;
        let backend = Backend::detect()?;
        let spec = backend.uninstall_spec(packages);
        if dry_run {
            return print_dry_run(&backend, &spec);
        }
        execute_spec(backend, spec)
    }
}

fn upgrade(packages: &[String], dry_run: bool, kind: PackageKind) -> Result<ExitCode, UpkgError> {
    #[cfg(target_os = "macos")]
    {
        if dry_run {
            native::print_upgrade_dry_run(packages, kind)?;
            return Ok(ExitCode::SUCCESS);
        }

        native::upgrade_native(packages, kind)?;
        Ok(ExitCode::SUCCESS)
    }

    #[cfg(not(target_os = "macos"))]
    {
        reject_app_kind(kind)?;
        let backend = Backend::detect()?;
        let spec = backend.upgrade_spec(packages);
        if dry_run {
            return print_dry_run(&backend, &spec);
        }
        execute_spec(backend, spec)
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
        execute_spec(backend, spec)
    }
}

#[cfg(not(target_os = "macos"))]
fn print_dry_run(backend: &Backend, spec: &backend::CommandSpec) -> Result<ExitCode, UpkgError> {
    println!("backend: {}", backend.name());
    println!("dry-run: {}", spec.render());
    Ok(ExitCode::SUCCESS)
}

fn self_upgrade(dry_run: bool) -> Result<ExitCode, UpkgError> {
    self_upgrade::run(dry_run)?;
    Ok(ExitCode::SUCCESS)
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
