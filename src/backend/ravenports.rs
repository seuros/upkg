use crate::backend::CommandSpec;
use crate::error::UpkgError;
use std::path::Path;

const RVN_PATH: &str = "/raven/sbin/rvn";

pub fn is_available() -> bool {
    Path::new(RVN_PATH).is_file()
}

pub fn install_spec(packages: &[String]) -> CommandSpec {
    let mut args = vec!["install".to_string(), "-y".to_string()];
    args.extend(packages.iter().cloned());
    CommandSpec::new(RVN_PATH, args)
}

pub fn uninstall_spec(packages: &[String]) -> CommandSpec {
    let mut args = vec!["remove".to_string(), "-y".to_string()];
    args.extend(packages.iter().cloned());
    CommandSpec::new(RVN_PATH, args)
}

pub fn upgrade_spec(packages: &[String]) -> CommandSpec {
    let mut args = vec!["upgrade".to_string(), "-y".to_string()];
    args.extend(packages.iter().cloned());
    CommandSpec::new(RVN_PATH, args)
}

pub fn list_spec() -> CommandSpec {
    CommandSpec::new(RVN_PATH, vec!["info".to_string(), "-a".to_string()])
}

pub fn search_spec(query: &str, exact: bool) -> Result<CommandSpec, UpkgError> {
    let mut args = vec!["search".to_string()];
    if exact {
        args.push("-e".to_string());
    }
    args.push(query.to_string());
    Ok(CommandSpec::new(RVN_PATH, args))
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
