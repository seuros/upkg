use crate::backend::CommandSpec;
use crate::error::UpkgError;

fn sudo_pkg(args: &[&str], packages: &[String]) -> CommandSpec {
    CommandSpec::with_packages("sudo", &[&["pkg"], args].concat(), packages)
}

pub fn install_spec(packages: &[String]) -> CommandSpec {
    sudo_pkg(&["install", "-y"], packages)
}

pub fn uninstall_spec(packages: &[String]) -> CommandSpec {
    sudo_pkg(&["delete", "-y"], packages)
}

pub fn reinstall_spec(packages: &[String]) -> CommandSpec {
    sudo_pkg(&["install", "-f", "-y"], packages)
}

pub fn list_spec() -> CommandSpec {
    CommandSpec::with_packages("pkg", &["info"], &[])
}

pub fn upgrade_spec(packages: &[String]) -> CommandSpec {
    sudo_pkg(&["upgrade", "-y"], packages)
}

pub fn search_spec(query: &str, exact: bool) -> Result<CommandSpec, UpkgError> {
    let args: &[&str] = if exact {
        &["search", "-e", query]
    } else {
        &["search", query]
    };
    Ok(CommandSpec::with_packages("pkg", args, &[]))
}

#[cfg(test)]
mod tests;
