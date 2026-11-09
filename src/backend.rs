#[cfg(target_os = "android")]
pub mod android;
#[cfg(any(target_os = "freebsd", target_os = "dragonfly"))]
pub mod freebsd;
#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub mod ravenports;
#[cfg(target_os = "windows")]
pub mod windows;

use upkg::catalog;
pub use upkg::command::CommandSpec;
#[cfg(any(target_os = "android", target_os = "windows"))]
pub use upkg::command::command_exists;
use upkg::host::Platform;

#[cfg(target_os = "linux")]
use linux::LinuxSpecs;

use crate::error::UpkgError;

pub enum Backend {
    #[cfg(target_os = "android")]
    Android(android::AndroidManager),
    #[cfg(target_os = "linux")]
    Linux(linux::LinuxManager),
    #[cfg(target_os = "windows")]
    Windows(windows::WindowsManager),
    #[cfg(any(target_os = "freebsd", target_os = "dragonfly"))]
    FreeBsd,
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    Ravenports,
}

/// A manager whose program name is also the binary for every verb (install,
/// uninstall, ...), with no `sudo` wrapper and no alternate binary per verb.
/// Android's `pkg`/`apt` and Windows' `winget`/`choco` fit this shape; Linux
/// backends that shell out through `sudo <name> ...` don't and keep their own
/// `spec` helper instead of implementing this trait.
#[cfg(any(target_os = "windows", target_os = "android"))]
pub(crate) trait PackageManager {
    fn name(&self) -> &'static str;

    /// `CommandSpec::with_packages` against this manager's own program name.
    fn packages_spec(&self, args: &[&str], packages: &[String]) -> CommandSpec {
        CommandSpec::with_packages(self.name(), args, packages)
    }
}

impl Backend {
    pub fn detect() -> Result<Self, UpkgError> {
        // Ravenports is cross-platform (DragonFlyBSD, FreeBSD, Linux, Solaris).
        // Check it first so users who installed Ravenports get it regardless of OS.
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        if ravenports::is_available() {
            return Ok(Self::Ravenports);
        }

        #[cfg(target_os = "android")]
        {
            Ok(Self::Android(android::detect()?))
        }

        #[cfg(target_os = "linux")]
        {
            Ok(Self::Linux(linux::detect()?))
        }

        #[cfg(target_os = "windows")]
        {
            Ok(Self::Windows(windows::detect()?))
        }

        #[cfg(any(target_os = "freebsd", target_os = "dragonfly"))]
        {
            Ok(Self::FreeBsd)
        }

        #[cfg(not(any(
            target_os = "android",
            target_os = "linux",
            target_os = "windows",
            target_os = "freebsd",
            target_os = "dragonfly"
        )))]
        {
            Err(UpkgError::Unsupported("unsupported operating system"))
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            #[cfg(target_os = "android")]
            Self::Android(manager) => manager.name(),
            #[cfg(target_os = "linux")]
            Self::Linux(manager) => manager.name(),
            #[cfg(target_os = "windows")]
            Self::Windows(manager) => manager.name(),
            #[cfg(any(target_os = "freebsd", target_os = "dragonfly"))]
            Self::FreeBsd => "pkg",
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            Self::Ravenports => "rvn",
        }
    }

    /// Stable key used to look the backend up in the alias catalog.
    ///
    /// Distinct from `name()` only where the binary name would collide: Termux
    /// `pkg` uses Debian-style names while FreeBSD `pkg` does not, so they get
    /// separate keys (`termux` vs `freebsd`).
    pub fn catalog_key(&self) -> &'static str {
        match self {
            #[cfg(target_os = "android")]
            Self::Android(manager) => manager.catalog_key(),
            #[cfg(target_os = "linux")]
            Self::Linux(manager) => manager.name(),
            #[cfg(target_os = "windows")]
            Self::Windows(manager) => manager.name(),
            #[cfg(any(target_os = "freebsd", target_os = "dragonfly"))]
            Self::FreeBsd => "freebsd",
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            Self::Ravenports => "rvn",
        }
    }

    /// Resolves `packages` through the alias catalog, keyed by this backend's
    /// manager. Shared by every `*_spec` method below.
    fn resolved_packages(&self, packages: &[String]) -> Vec<String> {
        catalog::resolve(self.catalog_key(), packages)
    }

    /// The native name `package` resolves to on this backend: Termux's
    /// `check` asks for one name at a time.
    #[cfg(target_os = "android")]
    pub fn resolve(&self, package: &str) -> String {
        catalog::resolve_one(self.catalog_key(), package)
    }

    /// The host library's platform, for `check`'s queries
    /// ([`Host::status`](upkg::host::Host::status)). Termux, winget, choco
    /// and rvn have none.
    pub const fn host_platform(&self) -> Result<Platform, UpkgError> {
        match self {
            #[cfg(target_os = "android")]
            Self::Android(_) => Err(UpkgError::Unsupported(
                "Termux is checked by the CLI, not the host library",
            )),
            // Rolling or not only changes how a system upgrades.
            #[cfg(target_os = "linux")]
            Self::Linux(manager) => Ok(Platform::Linux {
                manager: *manager,
                rolling: false,
            }),
            #[cfg(target_os = "windows")]
            Self::Windows(_) => Err(UpkgError::Unsupported(
                "check is not supported by winget or choco",
            )),
            // pkgbase or not only changes how a system upgrades.
            #[cfg(target_os = "freebsd")]
            Self::FreeBsd => Ok(Platform::FreeBsd { pkgbase: false }),
            #[cfg(target_os = "dragonfly")]
            Self::FreeBsd => Ok(Platform::DragonFly),
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            Self::Ravenports => Err(UpkgError::Unsupported("check is not supported by rvn")),
        }
    }

    pub fn install_spec(&self, packages: &[String]) -> CommandSpec {
        let packages = self.resolved_packages(packages);
        match self {
            #[cfg(target_os = "android")]
            Self::Android(manager) => manager.install_spec(&packages),
            #[cfg(target_os = "linux")]
            Self::Linux(manager) => manager.install_spec(&packages),
            #[cfg(target_os = "windows")]
            Self::Windows(manager) => manager.install_spec(&packages),
            #[cfg(any(target_os = "freebsd", target_os = "dragonfly"))]
            Self::FreeBsd => freebsd::install_spec(&packages),
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            Self::Ravenports => ravenports::install_spec(&packages),
        }
    }

    pub fn uninstall_spec(&self, packages: &[String]) -> CommandSpec {
        let packages = self.resolved_packages(packages);
        match self {
            #[cfg(target_os = "android")]
            Self::Android(manager) => manager.uninstall_spec(&packages),
            #[cfg(target_os = "linux")]
            Self::Linux(manager) => manager.uninstall_spec(&packages),
            #[cfg(target_os = "windows")]
            Self::Windows(manager) => manager.uninstall_spec(&packages),
            #[cfg(any(target_os = "freebsd", target_os = "dragonfly"))]
            Self::FreeBsd => freebsd::uninstall_spec(&packages),
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            Self::Ravenports => ravenports::uninstall_spec(&packages),
        }
    }

    pub fn reinstall_spec(&self, packages: &[String]) -> Result<CommandSpec, UpkgError> {
        let packages = self.resolved_packages(packages);
        match self {
            #[cfg(target_os = "android")]
            Self::Android(manager) => Ok(manager.reinstall_spec(&packages)),
            #[cfg(target_os = "linux")]
            Self::Linux(manager) => Ok(manager.reinstall_spec(&packages)),
            #[cfg(target_os = "windows")]
            Self::Windows(manager) => manager.reinstall_spec(&packages),
            #[cfg(any(target_os = "freebsd", target_os = "dragonfly"))]
            Self::FreeBsd => Ok(freebsd::reinstall_spec(&packages)),
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            Self::Ravenports => Err(UpkgError::Unsupported("reinstall is not supported by rvn")),
        }
    }

    pub fn upgrade_spec(&self, packages: &[String]) -> CommandSpec {
        let packages = self.resolved_packages(packages);
        match self {
            #[cfg(target_os = "android")]
            Self::Android(manager) => manager.upgrade_spec(&packages),
            #[cfg(target_os = "linux")]
            Self::Linux(manager) => manager.upgrade_spec(&packages),
            #[cfg(target_os = "windows")]
            Self::Windows(manager) => manager.upgrade_spec(&packages),
            #[cfg(any(target_os = "freebsd", target_os = "dragonfly"))]
            Self::FreeBsd => freebsd::upgrade_spec(&packages),
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            Self::Ravenports => ravenports::upgrade_spec(&packages),
        }
    }

    /// An index refresh to run before installing, when the manager has no
    /// package lists yet (fresh containers).
    pub fn refresh_spec(&self) -> Option<CommandSpec> {
        match self {
            #[cfg(target_os = "linux")]
            Self::Linux(manager) => manager.refresh_spec(),
            #[allow(unreachable_patterns)]
            _ => None,
        }
    }

    pub fn search_spec(&self, query: &str, exact: bool) -> Result<CommandSpec, UpkgError> {
        match self {
            #[cfg(target_os = "android")]
            Self::Android(manager) => manager.search_spec(query, exact),
            #[cfg(target_os = "linux")]
            Self::Linux(manager) => manager.search_spec(query, exact),
            #[cfg(target_os = "windows")]
            Self::Windows(manager) => manager.search_spec(query, exact),
            #[cfg(any(target_os = "freebsd", target_os = "dragonfly"))]
            Self::FreeBsd => freebsd::search_spec(query, exact),
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            Self::Ravenports => ravenports::search_spec(query, exact),
        }
    }

    pub fn list_spec(&self) -> CommandSpec {
        match self {
            #[cfg(target_os = "android")]
            Self::Android(manager) => manager.list_spec(),
            #[cfg(target_os = "linux")]
            Self::Linux(manager) => manager.list_spec(),
            #[cfg(target_os = "windows")]
            Self::Windows(manager) => manager.list_spec(),
            #[cfg(any(target_os = "freebsd", target_os = "dragonfly"))]
            Self::FreeBsd => freebsd::list_spec(),
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            Self::Ravenports => ravenports::list_spec(),
        }
    }
}

#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;
