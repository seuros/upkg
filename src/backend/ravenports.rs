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
mod tests {
    use super::*;

    #[test]
    fn install_spec_generates_correct_command() {
        let packages = vec!["git".into()];
        let spec = install_spec(&packages);

        assert_eq!(spec.command(), RVN_PATH);
        let args: Vec<&str> = spec.args().iter().map(|s| s.as_str()).collect();
        assert_eq!(args, vec!["install", "-y", "git"]);
    }

    #[test]
    fn install_spec_handles_multiple_packages() {
        let packages = vec!["vim".into(), "curl".into(), "wget".into()];
        let spec = install_spec(&packages);

        let args: Vec<&str> = spec.args().iter().map(|s| s.as_str()).collect();
        assert_eq!(args, vec!["install", "-y", "vim", "curl", "wget"]);
    }

    #[test]
    fn install_spec_handles_empty_packages() {
        let packages: Vec<String> = vec![];
        let spec = install_spec(&packages);

        assert_eq!(spec.command(), RVN_PATH);
        let args: Vec<&str> = spec.args().iter().map(|s| s.as_str()).collect();
        assert_eq!(args, vec!["install", "-y"]);
    }

    #[test]
    fn uninstall_spec_generates_correct_command() {
        let packages = vec!["git".into()];
        let spec = uninstall_spec(&packages);

        assert_eq!(spec.command(), RVN_PATH);
        let args: Vec<&str> = spec.args().iter().map(|s| s.as_str()).collect();
        assert_eq!(args, vec!["remove", "-y", "git"]);
    }

    #[test]
    fn search_spec_non_exact() {
        let spec = search_spec("ripgrep", false).unwrap();
        assert_eq!(spec.command(), RVN_PATH);
        let args: Vec<&str> = spec.args().iter().map(|s| s.as_str()).collect();
        assert_eq!(args, vec!["search", "ripgrep"]);
    }

    #[test]
    fn search_spec_exact() {
        let spec = search_spec("git", true).unwrap();
        let args: Vec<&str> = spec.args().iter().map(|s| s.as_str()).collect();
        assert_eq!(args, vec!["search", "-e", "git"]);
    }

    #[test]
    fn upgrade_spec_handles_empty_packages() {
        let packages: Vec<String> = vec![];
        let spec = upgrade_spec(&packages);

        assert_eq!(spec.command(), RVN_PATH);
        let args: Vec<&str> = spec.args().iter().map(|s| s.as_str()).collect();
        assert_eq!(args, vec!["upgrade", "-y"]);
    }
}
