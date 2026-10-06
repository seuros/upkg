use std::ffi::OsString;

use crate::error::UpkgError;

mod parser;

#[derive(Debug)]
pub struct Cli {
    pub command: CommandKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageKind {
    Auto,
    App,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageAction {
    Install,
    Uninstall,
    Upgrade,
    Reinstall,
}

#[derive(Debug)]
pub enum CommandKind {
    Package {
        action: PackageAction,
        packages: Vec<String>,
        dry_run: bool,
        kind: PackageKind,
    },
    List,
    Search {
        query: String,
        exact: bool,
        kind: PackageKind,
        refresh: bool,
    },
    Print(String),
    SelfUpgrade {
        dry_run: bool,
    },
    Shaman,
}

impl Cli {
    pub fn parse<I>(args: I) -> Result<Self, UpkgError>
    where
        I: IntoIterator,
        I::Item: Into<OsString>,
    {
        parser::parse(args.into_iter().map(Into::into))
    }
}

#[cfg(test)]
mod tests;
