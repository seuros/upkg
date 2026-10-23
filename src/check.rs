//! `upkg check`: whether packages are installed, without changing anything.

use std::process::ExitCode;
#[cfg(not(target_os = "macos"))]
use std::process::Stdio;

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

/// Runs the backend's installed-query for `requested`; stderr passes through.
#[cfg(not(target_os = "macos"))]
pub fn on_backend(backend: &Backend, requested: &str) -> Result<Checked, UpkgError> {
    let native = backend.resolve(requested);
    let query = backend.installed_query(&native)?;
    let rendered = query.spec.render();
    let output = query
        .spec
        .into_command()
        .stderr(Stdio::inherit())
        .output()
        .map_err(|err| std::io::Error::new(err.kind(), format!("`{rendered}`: {err}")))?;
    Ok(Checked {
        requested: requested.to_string(),
        installed: query
            .answer
            .installed(output.status.success(), &output.stdout),
        native,
    })
}

#[cfg(test)]
mod tests;
