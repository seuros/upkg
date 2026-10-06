use crate::backend::{CommandSpec, command_exists};
use crate::error::UpkgError;

pub enum WindowsManager {
    Winget,
    Choco,
}

pub fn detect() -> Result<WindowsManager, UpkgError> {
    if command_exists("winget") {
        return Ok(WindowsManager::Winget);
    }
    if command_exists("choco") {
        return Ok(WindowsManager::Choco);
    }

    Err(UpkgError::Unsupported(
        "no supported package manager found (winget/choco)",
    ))
}

impl WindowsManager {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Winget => "winget",
            Self::Choco => "choco",
        }
    }

    pub fn install_spec(&self, packages: &[String]) -> CommandSpec {
        let args: &[&str] = match self {
            Self::Winget => &[
                "install",
                "--silent",
                "--accept-source-agreements",
                "--accept-package-agreements",
            ],
            Self::Choco => &["install", "-y"],
        };
        CommandSpec::with_packages(self.name(), args, packages)
    }

    pub fn uninstall_spec(&self, packages: &[String]) -> CommandSpec {
        let args: &[&str] = match self {
            Self::Winget => &["uninstall", "--silent", "--accept-source-agreements"],
            Self::Choco => &["uninstall", "-y"],
        };
        CommandSpec::with_packages(self.name(), args, packages)
    }

    pub fn reinstall_spec(&self, packages: &[String]) -> Result<CommandSpec, UpkgError> {
        match self {
            Self::Winget => Err(UpkgError::Unsupported(
                "reinstall is not supported by winget; uninstall and install instead",
            )),
            Self::Choco => Ok(CommandSpec::with_packages(
                self.name(),
                &["install", "-y", "--force"],
                packages,
            )),
        }
    }

    pub fn upgrade_spec(&self, packages: &[String]) -> CommandSpec {
        let args: &[&str] = match self {
            Self::Winget => &[
                "upgrade",
                "--silent",
                "--accept-source-agreements",
                "--accept-package-agreements",
            ],
            Self::Choco => &["upgrade", "-y"],
        };
        CommandSpec::with_packages(self.name(), args, packages)
    }

    pub fn search_spec(&self, query: &str, exact: bool) -> Result<CommandSpec, UpkgError> {
        let args: &[&str] = match (self, exact) {
            (Self::Winget, true) => &["search", "-e", query],
            (Self::Choco, true) => &["search", query, "-e"],
            (_, false) => &["search", query],
        };
        Ok(CommandSpec::with_packages(self.name(), args, &[]))
    }

    pub fn list_spec(&self) -> CommandSpec {
        let args: &[&str] = match self {
            Self::Winget => &["list"],
            Self::Choco => &["list", "--local-only"],
        };
        CommandSpec::with_packages(self.name(), args, &[])
    }
}

#[cfg(test)]
mod tests;
