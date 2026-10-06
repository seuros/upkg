use crate::backend::CommandSpec;
use crate::error::UpkgError;
use std::path::Path;

const RVN_PATH: &str = "/raven/sbin/rvn";

pub fn is_available() -> bool {
    Path::new(RVN_PATH).is_file()
}

pub fn install_spec(packages: &[String]) -> CommandSpec {
    CommandSpec::with_packages(RVN_PATH, &["install", "-y"], packages)
}

pub fn uninstall_spec(packages: &[String]) -> CommandSpec {
    CommandSpec::with_packages(RVN_PATH, &["remove", "-y"], packages)
}

pub fn upgrade_spec(packages: &[String]) -> CommandSpec {
    CommandSpec::with_packages(RVN_PATH, &["upgrade", "-y"], packages)
}

pub fn list_spec() -> CommandSpec {
    CommandSpec::with_packages(RVN_PATH, &["info", "-a"], &[])
}

pub fn search_spec(query: &str, exact: bool) -> Result<CommandSpec, UpkgError> {
    let args: &[&str] = if exact {
        &["search", "-e", query]
    } else {
        &["search", query]
    };
    Ok(CommandSpec::with_packages(RVN_PATH, args, &[]))
}

#[cfg(test)]
mod tests;
