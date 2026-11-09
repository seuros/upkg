use std::ffi::OsString;
use std::fs;
use std::path::Path;

pub use upkg::host::LinuxManager;

use crate::backend::CommandSpec;
use crate::error::UpkgError;

/// Where apt keeps downloaded package lists; container images ship it empty.
const APT_LISTS: &str = "/var/lib/apt/lists";

fn reject_exact_if(exact: bool, manager: &'static str) -> Result<(), UpkgError> {
    if exact {
        return Err(UpkgError::Unsupported(match manager {
            "apt" => "--exact is not supported by apt search",
            "dnf" => "--exact is not supported by dnf search",
            "yum" => "--exact is not supported by yum search",
            "zypper" => "--exact is not supported by zypper search",
            _ => "--exact is not supported by this backend",
        }));
    }
    Ok(())
}

fn escape_ere(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(
            c,
            '\\' | '.' | '+' | '*' | '?' | '(' | ')' | '|' | '[' | ']' | '{' | '}' | '^' | '$'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn anchor_if_exact(query: &str, exact: bool) -> String {
    if exact {
        format!("^{}$", escape_ere(query))
    } else {
        query.to_string()
    }
}

/// The running distribution's manager; detection lives with the host API.
pub fn detect() -> Result<LinuxManager, UpkgError> {
    Ok(LinuxManager::detect()?)
}

/// Whether apt has fetched package lists at least once.
fn has_package_lists(dir: &Path) -> bool {
    fs::read_dir(dir)
        .is_ok_and(|entries| lists_packages(entries.flatten().map(|entry| entry.file_name())))
}

/// apt names lists `<repo>_Packages`, plus `.lz4`/`.gz` when compressed.
fn lists_packages(names: impl IntoIterator<Item = OsString>) -> bool {
    names
        .into_iter()
        .any(|name| name.to_string_lossy().contains("_Packages"))
}

/// `sudo <manager> args... packages...`; opkg runs as root already.
fn spec(manager: LinuxManager, args: &[&str], packages: &[String]) -> CommandSpec {
    match manager {
        LinuxManager::Opkg => CommandSpec::with_packages("opkg", args, packages),
        _ => CommandSpec::with_packages("sudo", &[&[manager.name()], args].concat(), packages),
    }
}

/// The package-operation commands of a Linux manager.
pub trait LinuxSpecs {
    fn install_spec(&self, packages: &[String]) -> CommandSpec;
    fn refresh_spec(&self) -> Option<CommandSpec>;
    fn uninstall_spec(&self, packages: &[String]) -> CommandSpec;
    fn reinstall_spec(&self, packages: &[String]) -> CommandSpec;
    fn upgrade_spec(&self, packages: &[String]) -> CommandSpec;
    fn search_spec(&self, query: &str, exact: bool) -> Result<CommandSpec, UpkgError>;
    fn list_spec(&self) -> CommandSpec;
}

impl LinuxSpecs for LinuxManager {
    fn install_spec(&self, packages: &[String]) -> CommandSpec {
        let args: &[&str] = match self {
            Self::Apt | Self::Dnf | Self::Yum => &["install", "-y"],
            Self::Pacman => &["-S", "--noconfirm"],
            Self::Zypper => &["--non-interactive", "install"],
            Self::Opkg => &["install"],
            // Alpine images ship without an index; fetch it on every install.
            Self::Apk => &["add", "--update-cache"],
        };
        spec(*self, args, packages)
    }

    /// `apt update` when no package lists were ever fetched. Other managers
    /// refresh as part of the install (apk) or need no index.
    fn refresh_spec(&self) -> Option<CommandSpec> {
        match self {
            Self::Apt if !has_package_lists(Path::new(APT_LISTS)) => {
                Some(spec(*self, &["update"], &[]))
            }
            _ => None,
        }
    }

    fn uninstall_spec(&self, packages: &[String]) -> CommandSpec {
        let args: &[&str] = match self {
            Self::Apt | Self::Dnf | Self::Yum => &["remove", "-y"],
            Self::Pacman => &["-R", "--noconfirm"],
            Self::Zypper => &["--non-interactive", "remove"],
            Self::Opkg => &["remove"],
            Self::Apk => &["del"],
        };
        spec(*self, args, packages)
    }

    fn reinstall_spec(&self, packages: &[String]) -> CommandSpec {
        let args: &[&str] = match self {
            Self::Apt => &["install", "--reinstall", "-y"],
            Self::Dnf | Self::Yum => &["reinstall", "-y"],
            // -S reinstalls packages that are already up to date.
            Self::Pacman => &["-S", "--noconfirm"],
            Self::Zypper => &["--non-interactive", "install", "--force"],
            Self::Opkg => &["install", "--force-reinstall"],
            Self::Apk => &["fix", "--reinstall"],
        };
        spec(*self, args, packages)
    }

    fn upgrade_spec(&self, packages: &[String]) -> CommandSpec {
        let args: &[&str] = match self {
            Self::Apt if packages.is_empty() => &["upgrade", "-y"],
            Self::Apt => &["install", "--only-upgrade", "-y"],
            Self::Dnf => &["upgrade", "-y"],
            Self::Yum => &["update", "-y"],
            Self::Pacman if packages.is_empty() => &["-Syu", "--noconfirm"],
            Self::Pacman => &["-S", "--noconfirm"],
            Self::Zypper => &["--non-interactive", "update"],
            Self::Opkg => &["upgrade"],
            Self::Apk if packages.is_empty() => &["upgrade", "--update-cache"],
            Self::Apk => &["add", "--update-cache", "--upgrade"],
        };
        spec(*self, args, packages)
    }

    fn search_spec(&self, query: &str, exact: bool) -> Result<CommandSpec, UpkgError> {
        let args = match self {
            Self::Pacman => vec!["-Ss".to_string(), anchor_if_exact(query, exact)],
            Self::Opkg => vec!["find".to_string(), anchor_if_exact(query, exact)],
            Self::Apk if exact => vec![
                "search".to_string(),
                "--exact".to_string(),
                query.to_string(),
            ],
            _ => {
                reject_exact_if(exact, self.name())?;
                vec!["search".to_string(), query.to_string()]
            }
        };
        Ok(CommandSpec::new(self.name(), args))
    }

    fn list_spec(&self) -> CommandSpec {
        match self {
            Self::Apt => CommandSpec::new("dpkg", vec!["--get-selections".into()]),
            Self::Dnf => CommandSpec::new("dnf", vec!["list".into(), "--installed".into()]),
            Self::Yum => CommandSpec::new("yum", vec!["list".into(), "installed".into()]),
            Self::Pacman => CommandSpec::new("pacman", vec!["-Q".into()]),
            Self::Zypper => {
                CommandSpec::new("zypper", vec!["se".into(), "--installed-only".into()])
            }
            Self::Opkg => CommandSpec::new("opkg", vec!["list-installed".into()]),
            Self::Apk => CommandSpec::new("apk", vec!["info".into()]),
        }
    }
}

#[cfg(test)]
mod tests;
