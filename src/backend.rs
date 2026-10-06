#[cfg(target_os = "android")]
pub mod android;
pub mod catalog;
#[cfg(any(target_os = "freebsd", target_os = "dragonfly"))]
pub mod freebsd;
#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub mod ravenports;
#[cfg(target_os = "windows")]
pub mod windows;

#[cfg(not(target_os = "macos"))]
use std::env;
#[cfg(not(target_os = "macos"))]
use std::path::Path;
#[cfg(not(target_os = "macos"))]
use std::process::Command;

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

/// How a command that needs root is run.
#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Escalation {
    /// Already root (containers, CI): run the command as is.
    Root,
    Sudo,
    /// No sudo but doas, as on Alpine and the BSDs.
    Doas,
}

#[cfg(unix)]
impl Escalation {
    /// Root, else sudo, else doas. Falls back to sudo when neither exists,
    /// so the error names the missing tool.
    pub fn detect() -> Self {
        if is_root() {
            Self::Root
        } else if !command_exists("sudo") && command_exists("doas") {
            Self::Doas
        } else {
            Self::Sudo
        }
    }
}

#[cfg(unix)]
fn is_root() -> bool {
    Command::new("id")
        .arg("-u")
        .output()
        .is_ok_and(|output| output.stdout.trim_ascii() == b"0")
}

#[derive(Debug)]
pub struct CommandSpec {
    program: String,
    args: Vec<String>,
}

impl CommandSpec {
    pub fn new(program: impl Into<String>, args: impl Into<Vec<String>>) -> Self {
        Self {
            program: program.into(),
            args: args.into(),
        }
    }

    pub fn with_packages(program: impl Into<String>, args: &[&str], packages: &[String]) -> Self {
        let mut argv: Vec<String> = args.iter().map(ToString::to_string).collect();
        argv.extend(packages.iter().cloned());
        Self::new(program, argv)
    }

    pub fn render(&self) -> String {
        format!("{} {}", self.program, self.args.join(" "))
            .trim()
            .to_string()
    }

    /// Rewrites a `sudo` prefix: dropped for root, swapped for doas.
    #[cfg(unix)]
    pub fn escalate(self, escalation: Escalation) -> Self {
        if self.program != "sudo" {
            return self;
        }
        match escalation {
            Escalation::Sudo => self,
            Escalation::Doas => Self::new("doas", self.args),
            Escalation::Root => {
                let mut args = self.args.into_iter();
                match args.next() {
                    Some(program) => Self::new(program, args.collect::<Vec<_>>()),
                    None => Self::new("sudo", Vec::<String>::new()),
                }
            }
        }
    }

    /// The command as the current user can run it.
    pub fn for_current_user(self) -> Self {
        #[cfg(unix)]
        {
            self.escalate(Escalation::detect())
        }
        #[cfg(not(unix))]
        {
            self
        }
    }

    #[cfg(not(target_os = "macos"))]
    pub fn into_command(self) -> Command {
        let mut command = Command::new(self.program);
        command.args(self.args);
        command
    }

    #[cfg(test)]
    pub fn command(&self) -> &str {
        &self.program
    }

    #[cfg(test)]
    pub fn args(&self) -> &[String] {
        &self.args
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

    pub fn install_spec(&self, packages: &[String]) -> CommandSpec {
        let packages = catalog::resolve(self.catalog_key(), packages);
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
        let packages = catalog::resolve(self.catalog_key(), packages);
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
        let packages = catalog::resolve(self.catalog_key(), packages);
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
        let packages = catalog::resolve(self.catalog_key(), packages);
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

#[cfg(not(target_os = "macos"))]
pub fn command_exists(name: &str) -> bool {
    if name.contains(std::path::MAIN_SEPARATOR) {
        return Path::new(name).is_file();
    }

    let Some(paths) = env::var_os("PATH") else {
        return false;
    };

    env::split_paths(&paths).any(|path| {
        let candidate = path.join(name);
        if candidate.is_file() {
            return true;
        }

        #[cfg(windows)]
        {
            ["exe", "cmd", "bat"]
                .iter()
                .any(|ext| path.join(format!("{name}.{ext}")).is_file())
        }

        #[cfg(not(windows))]
        {
            false
        }
    })
}

#[cfg(test)]
mod tests;
