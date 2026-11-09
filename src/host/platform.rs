//! Which native package manager runs this host, read off the running system.

use super::{Error, Runner};
use crate::command::CommandSpec;

/// The Linux package managers upkg speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinuxManager {
    Apt,
    Dnf,
    Yum,
    Pacman,
    Zypper,
    Opkg,
    Apk,
}

impl LinuxManager {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Apt => "apt",
            Self::Dnf => "dnf",
            Self::Yum => "yum",
            Self::Pacman => "pacman",
            Self::Zypper => "zypper",
            Self::Opkg => "opkg",
            Self::Apk => "apk",
        }
    }

    /// The manager of the running system, from `/etc/os-release`.
    ///
    /// # Errors
    ///
    /// The file cannot be read, or names a distribution upkg doesn't know.
    pub fn detect() -> Result<Self, Error> {
        let os_release = std::fs::read_to_string(OS_RELEASE).map_err(|source| Error::Io {
            context: OS_RELEASE.to_string(),
            source,
        })?;
        Self::from_os_release(&os_release, crate::command::command_exists("dnf"))
    }

    /// The manager for an `/etc/os-release`; `has_dnf` picks dnf over yum.
    ///
    /// # Errors
    ///
    /// [`Error::Unsupported`] for a distribution upkg doesn't know.
    pub fn from_os_release(os_release: &str, has_dnf: bool) -> Result<Self, Error> {
        let release = OsRelease::parse(os_release);
        if release.is_like(&["debian", "ubuntu"]) {
            Ok(Self::Apt)
        } else if release.is_like(&["fedora", "rhel", "centos"]) {
            Ok(if has_dnf { Self::Dnf } else { Self::Yum })
        } else if release.is_like(&["arch"]) {
            Ok(Self::Pacman)
        } else if release.is_like(&["suse", "opensuse"]) || release.id.starts_with("opensuse") {
            Ok(Self::Zypper)
        } else if release.is_like(&["openwrt"]) {
            Ok(Self::Opkg)
        } else if release.is_like(&["alpine"]) {
            Ok(Self::Apk)
        } else {
            Err(Error::Unsupported("unsupported Linux distribution"))
        }
    }
}

const OS_RELEASE: &str = "/etc/os-release";

/// The `ID` and `ID_LIKE` of an os-release file, unquoted.
struct OsRelease {
    id: String,
    like: Vec<String>,
}

impl OsRelease {
    fn parse(text: &str) -> Self {
        let field = |name: &str| {
            text.lines()
                .filter_map(|line| line.trim().strip_prefix(name)?.strip_prefix('='))
                .map(|value| value.trim().trim_matches(['"', '\'']).to_string())
                .next_back()
                .unwrap_or_default()
        };
        Self {
            id: field("ID"),
            like: field("ID_LIKE")
                .split_whitespace()
                .map(ToString::to_string)
                .collect(),
        }
    }

    /// Whether `ID` or any `ID_LIKE` entry is one of `families`.
    fn is_like(&self, families: &[&str]) -> bool {
        std::iter::once(&self.id)
            .chain(&self.like)
            .any(|id| families.contains(&id.as_str()))
    }

    /// Rolling releases want a distribution upgrade on every update.
    fn is_rolling(&self) -> bool {
        matches!(
            self.id.as_str(),
            "opensuse-tumbleweed" | "opensuse-slowroll"
        )
    }
}

/// The host's package manager, plus the facts that change how it is driven.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Linux {
        manager: LinuxManager,
        /// A rolling release (openSUSE Tumbleweed, Slowroll): zypper updates
        /// with `dist-upgrade` instead of `update`.
        rolling: bool,
    },
    FreeBsd {
        /// Base is installed from packages (pkgbase), so `pkg upgrade`
        /// updates it and freebsd-update refuses to run.
        pkgbase: bool,
    },
    /// `DragonFly` BSD: `pkg` for packages, base is rebuilt from source.
    DragonFly,
}

impl Platform {
    /// The package manager's name, as `upkg` prints it.
    #[must_use]
    pub const fn manager(&self) -> &'static str {
        match self {
            Self::Linux { manager, .. } => manager.name(),
            Self::FreeBsd { .. } | Self::DragonFly => "pkg",
        }
    }

    /// The key this platform's names go by in the alias catalog
    /// ([`crate::catalog`]): the Linux manager's name, `freebsd` for both
    /// BSDs (`DragonFly`'s `DPorts` keep FreeBSD's package names).
    #[must_use]
    pub const fn catalog_key(&self) -> &'static str {
        match self {
            Self::Linux { manager, .. } => manager.name(),
            Self::FreeBsd { .. } | Self::DragonFly => "freebsd",
        }
    }

    /// The running system's platform. Ravenports is not considered: it
    /// manages `/raven`, not the host.
    ///
    /// # Errors
    ///
    /// The OS is not Linux, FreeBSD or `DragonFly`, or its distribution is unknown.
    // Elsewhere the body is a bare `Err`, which neither reads `runner` nor
    // stops it from being const; the probing platforms need both.
    #[cfg_attr(
        not(any(target_os = "linux", target_os = "freebsd")),
        allow(unused_variables, clippy::missing_const_for_fn)
    )]
    pub fn detect(runner: &impl Runner) -> Result<Self, Error> {
        #[cfg(target_os = "linux")]
        {
            let os_release = runner
                .read_to_string(std::path::Path::new(OS_RELEASE))
                .map_err(|source| Error::Io {
                    context: OS_RELEASE.to_string(),
                    source,
                })?;
            Self::from_os_release(&os_release, runner.has_command("dnf"))
        }

        #[cfg(target_os = "freebsd")]
        {
            Ok(Self::FreeBsd {
                pkgbase: pkgbase(runner),
            })
        }

        #[cfg(target_os = "dragonfly")]
        {
            Ok(Self::DragonFly)
        }

        #[cfg(not(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly")))]
        {
            Err(Error::Unsupported(
                "host maintenance needs Linux, FreeBSD or DragonFly BSD",
            ))
        }
    }

    /// A Linux platform from an os-release file.
    ///
    /// # Errors
    ///
    /// [`Error::Unsupported`] for a distribution upkg doesn't know.
    pub fn from_os_release(os_release: &str, has_dnf: bool) -> Result<Self, Error> {
        Ok(Self::Linux {
            manager: LinuxManager::from_os_release(os_release, has_dnf)?,
            rolling: OsRelease::parse(os_release).is_rolling(),
        })
    }
}

/// The check freebsd-update(8) itself makes before refusing to run: base is
/// packaged when `/usr/bin/uname` belongs to a package.
#[cfg_attr(not(target_os = "freebsd"), allow(dead_code))]
fn pkgbase(runner: &impl Runner) -> bool {
    let probe = CommandSpec::new("pkg", vec!["which".into(), "/usr/bin/uname".into()]);
    runner
        .run(&probe)
        .is_ok_and(|execution| execution.success())
}

#[cfg(test)]
mod tests;
