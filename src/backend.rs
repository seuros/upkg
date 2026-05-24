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

#[cfg(any(target_os = "android", target_os = "linux", target_os = "windows"))]
use std::env;
#[cfg(any(target_os = "android", target_os = "linux", target_os = "windows"))]
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

    pub fn render(&self) -> String {
        format!("{} {}", self.program, self.args.join(" "))
            .trim()
            .to_string()
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

    pub fn install_spec(&self, packages: &[String]) -> CommandSpec {
        match self {
            #[cfg(target_os = "android")]
            Self::Android(manager) => manager.install_spec(packages),
            #[cfg(target_os = "linux")]
            Self::Linux(manager) => manager.install_spec(packages),
            #[cfg(target_os = "windows")]
            Self::Windows(manager) => manager.install_spec(packages),
            #[cfg(any(target_os = "freebsd", target_os = "dragonfly"))]
            Self::FreeBsd => freebsd::install_spec(packages),
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            Self::Ravenports => ravenports::install_spec(packages),
        }
    }

    pub fn uninstall_spec(&self, packages: &[String]) -> CommandSpec {
        match self {
            #[cfg(target_os = "android")]
            Self::Android(manager) => manager.uninstall_spec(packages),
            #[cfg(target_os = "linux")]
            Self::Linux(manager) => manager.uninstall_spec(packages),
            #[cfg(target_os = "windows")]
            Self::Windows(manager) => manager.uninstall_spec(packages),
            #[cfg(any(target_os = "freebsd", target_os = "dragonfly"))]
            Self::FreeBsd => freebsd::uninstall_spec(packages),
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            Self::Ravenports => ravenports::uninstall_spec(packages),
        }
    }

    pub fn upgrade_spec(&self, packages: &[String]) -> CommandSpec {
        match self {
            #[cfg(target_os = "android")]
            Self::Android(manager) => manager.upgrade_spec(packages),
            #[cfg(target_os = "linux")]
            Self::Linux(manager) => manager.upgrade_spec(packages),
            #[cfg(target_os = "windows")]
            Self::Windows(manager) => manager.upgrade_spec(packages),
            #[cfg(any(target_os = "freebsd", target_os = "dragonfly"))]
            Self::FreeBsd => freebsd::upgrade_spec(packages),
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            Self::Ravenports => ravenports::upgrade_spec(packages),
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

#[cfg(any(target_os = "android", target_os = "linux", target_os = "windows"))]
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
mod tests {
    use super::*;

    #[test]
    fn command_spec_new_creates_spec() {
        let spec = CommandSpec::new("ls", vec!["-la".to_string()]);
        assert_eq!(spec.command(), "ls");
        assert_eq!(spec.args(), &["-la"]);
    }

    #[test]
    fn command_spec_render_formats_command() {
        let spec = CommandSpec::new("git", vec!["status".to_string(), "--short".to_string()]);
        assert_eq!(spec.render(), "git status --short");
    }

    #[test]
    fn command_spec_render_handles_empty_args() {
        let spec = CommandSpec::new("pwd", Vec::<String>::new());
        assert_eq!(spec.render(), "pwd");
    }

    #[cfg(any(target_os = "android", target_os = "linux", target_os = "windows"))]
    #[test]
    fn command_exists_finds_common_command() {
        // 'sh' should exist on Linux/Android, 'cmd' on Windows
        #[cfg(unix)]
        let result = command_exists("sh");
        #[cfg(windows)]
        let result = command_exists("cmd");

        assert!(result);
    }

    #[cfg(any(target_os = "android", target_os = "linux", target_os = "windows"))]
    #[test]
    fn command_exists_rejects_nonexistent_command() {
        let result = command_exists("this_command_definitely_does_not_exist_12345");
        assert!(!result);
    }
}
