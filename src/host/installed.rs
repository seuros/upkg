//! "Is this package installed?": one read-only query per native name, the
//! same ones `upkg check` makes.

use super::{Execution, LinuxManager, Platform};
use crate::command::CommandSpec;
use crate::manifest::Package;

/// One package of a set, and whether the host has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageStatus {
    pub package: Package,
    pub installed: bool,
}

/// How a manager's query answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// Exit 0 means installed: `pkg info -e`, `rpm -q`, `pacman -Q`,
    /// `apk info -e`.
    ExitStatus,
    /// `dpkg-query`'s `${db:Status-Abbrev}`: a second character `i` means
    /// installed (`ii`, `hi`); `rc` is a removed package's leftover config.
    Dpkg,
    /// `opkg status`: a `Status:` line ending in `installed`, not
    /// `not-installed`.
    Opkg,
}

impl Answer {
    pub fn installed(self, execution: &Execution) -> bool {
        match self {
            Self::ExitStatus => execution.success(),
            Self::Dpkg => {
                execution.success()
                    && execution
                        .stdout
                        .lines()
                        .any(|line| line.as_bytes().get(1) == Some(&b'i'))
            }
            Self::Opkg => execution.stdout.lines().any(|line| {
                line.strip_prefix("Status:")
                    .and_then(|status| status.split_whitespace().last())
                    == Some("installed")
            }),
        }
    }
}

/// The query for `native` on `platform`. None of them needs root, and none
/// changes anything.
pub fn query(platform: Platform, native: &str) -> (CommandSpec, Answer) {
    let spec = |program: &str, args: &[&str]| {
        CommandSpec::with_packages(program, args, &[native.to_string()])
    };
    match platform {
        Platform::Linux { manager, .. } => match manager {
            LinuxManager::Apt => (
                spec(
                    "dpkg-query",
                    &["--show", "--showformat=${db:Status-Abbrev}\\n"],
                ),
                Answer::Dpkg,
            ),
            // dnf, yum and zypper all sit on the rpm database.
            LinuxManager::Dnf | LinuxManager::Yum | LinuxManager::Zypper => {
                (spec("rpm", &["-q"]), Answer::ExitStatus)
            }
            LinuxManager::Pacman => (spec("pacman", &["-Q"]), Answer::ExitStatus),
            LinuxManager::Opkg => (spec("opkg", &["status"]), Answer::Opkg),
            LinuxManager::Apk => (spec("apk", &["info", "-e"]), Answer::ExitStatus),
        },
        // A pkg never bootstrapped offers to fetch itself: it must decline,
        // even when the environment says yes to everything.
        Platform::FreeBsd { .. } | Platform::DragonFly => (
            spec("pkg", &["info", "-e"]).env("ASSUME_ALWAYS_YES", "no"),
            Answer::ExitStatus,
        ),
    }
}

#[cfg(test)]
mod tests;
