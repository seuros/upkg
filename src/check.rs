//! `upkg check`: whether packages are installed, without changing anything.

use std::process::ExitCode;

#[cfg(not(target_os = "macos"))]
use upkg::host::{CommandSpec, Execution, Host, PackageStatus, Runner, SystemRunner};
#[cfg(not(target_os = "macos"))]
use upkg::manifest::PackageSet;

#[cfg(not(target_os = "macos"))]
use crate::backend::Backend;
#[cfg(not(target_os = "macos"))]
use crate::error::UpkgError;

#[derive(Debug, PartialEq, Eq)]
pub struct Checked {
    pub requested: String,
    /// The name the native manager knows it by (after alias resolution).
    pub native: String,
    pub installed: bool,
}

impl Checked {
    /// `installed|missing <TAB> requested <TAB> native`, for scripts to parse.
    pub fn line(&self) -> String {
        let state = if self.installed {
            "installed"
        } else {
            "missing"
        };
        format!("{state}\t{}\t{}", self.requested, self.native)
    }
}

/// Prints every result and exits 1 when anything is missing.
pub fn report(results: &[Checked]) -> ExitCode {
    for checked in results {
        println!("{}", checked.line());
    }
    exit_code(results)
}

fn exit_code(results: &[Checked]) -> ExitCode {
    if results.iter().all(|checked| checked.installed) {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

/// Asks whether each of `requested` is installed, one result per name as
/// given: through the host library ([`Host::status`]) on Linux and the
/// BSDs, through dpkg on Termux.
#[cfg(not(target_os = "macos"))]
pub fn on_backend(backend: &Backend, requested: &[String]) -> Result<Vec<Checked>, UpkgError> {
    #[cfg(target_os = "android")]
    if let Backend::Android(_) = backend {
        return requested
            .iter()
            .map(|name| on_termux(backend, name))
            .collect();
    }
    let platform = backend.host_platform()?;
    let set = PackageSet::resolve(platform.catalog_key(), requested);
    let statuses = Host::new(platform, ShowStderr).status(&set)?;
    Ok(in_order(requested, &statuses))
}

/// `statuses` in the order (and with the repeats) of `requested`.
#[cfg(not(target_os = "macos"))]
fn in_order(requested: &[String], statuses: &[PackageStatus]) -> Vec<Checked> {
    requested
        .iter()
        .filter_map(|name| {
            statuses
                .iter()
                .find(|status| status.package.requested() == name)
        })
        .map(|status| Checked {
            requested: status.package.requested().to_string(),
            native: status.package.native().to_string(),
            installed: status.installed,
        })
        .collect()
}

/// The system's runner, showing each query's stderr once it ends: the
/// native tool's own explanation of a miss.
#[cfg(not(target_os = "macos"))]
struct ShowStderr;

#[cfg(not(target_os = "macos"))]
impl Runner for ShowStderr {
    fn run(&self, spec: &CommandSpec) -> std::io::Result<Execution> {
        let execution = SystemRunner::new().run(spec)?;
        eprint!("{}", execution.stderr);
        Ok(execution)
    }
}

/// Termux's dpkg, asked directly; stderr passes through.
#[cfg(target_os = "android")]
fn on_termux(backend: &Backend, requested: &str) -> Result<Checked, UpkgError> {
    use crate::backend::android;

    let native = backend.resolve(requested);
    let query = android::installed_query(&native);
    let rendered = query.render();
    let output = query
        .into_command()
        .stderr(std::process::Stdio::inherit())
        .output()
        .map_err(|err| std::io::Error::new(err.kind(), format!("`{rendered}`: {err}")))?;
    Ok(Checked {
        requested: requested.to_string(),
        installed: android::dpkg_installed(
            output.status.success(),
            &String::from_utf8_lossy(&output.stdout),
        ),
        native,
    })
}

#[cfg(test)]
mod tests;
