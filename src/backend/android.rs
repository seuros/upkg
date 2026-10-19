use crate::backend::{CommandSpec, PackageManager, command_exists};
use crate::error::UpkgError;

pub enum AndroidManager {
    Pkg,
    Apt,
}

pub fn detect() -> Result<AndroidManager, UpkgError> {
    if command_exists("pkg") {
        return Ok(AndroidManager::Pkg);
    }

    if command_exists("apt") {
        return Ok(AndroidManager::Apt);
    }

    Err(UpkgError::Unsupported(
        "no supported Android package manager found (pkg/apt)",
    ))
}

impl PackageManager for AndroidManager {
    fn name(&self) -> &'static str {
        match self {
            Self::Pkg => "pkg",
            Self::Apt => "apt",
        }
    }
}

impl AndroidManager {
    /// Alias-catalog key. Termux `pkg` ships Debian-style names, so it gets its
    /// own key rather than colliding with FreeBSD's `pkg`.
    pub fn catalog_key(&self) -> &'static str {
        match self {
            Self::Pkg => "termux",
            Self::Apt => "apt",
        }
    }

    pub fn install_spec(&self, packages: &[String]) -> CommandSpec {
        self.packages_spec(&["install", "-y"], packages)
    }

    pub fn uninstall_spec(&self, packages: &[String]) -> CommandSpec {
        let args: &[&str] = match self {
            Self::Pkg => &["uninstall", "-y"],
            Self::Apt => &["remove", "-y"],
        };
        self.packages_spec(args, packages)
    }

    pub fn reinstall_spec(&self, packages: &[String]) -> CommandSpec {
        let args: &[&str] = match self {
            Self::Pkg => &["reinstall"],
            Self::Apt => &["install", "--reinstall", "-y"],
        };
        self.packages_spec(args, packages)
    }

    pub fn search_spec(&self, query: &str, exact: bool) -> Result<CommandSpec, UpkgError> {
        if exact {
            return Err(UpkgError::Unsupported(match self {
                Self::Pkg => "--exact is not supported by pkg search (Termux)",
                Self::Apt => "--exact is not supported by apt search",
            }));
        }
        Ok(self.packages_spec(&["search", query], &[]))
    }

    pub fn upgrade_spec(&self, packages: &[String]) -> CommandSpec {
        let args: &[&str] = match self {
            Self::Apt if !packages.is_empty() => &["install", "--only-upgrade", "-y"],
            _ => &["upgrade", "-y"],
        };
        self.packages_spec(args, packages)
    }

    pub fn list_spec(&self) -> CommandSpec {
        let args: &[&str] = match self {
            Self::Pkg => &["list-installed"],
            Self::Apt => &["list", "--installed"],
        };
        self.packages_spec(args, &[])
    }
}

#[cfg(test)]
mod tests;
