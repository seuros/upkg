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
mod tests {
    use super::*;

    #[test]
    fn install_spec_generates_correct_command() {
        let packages = vec!["git".into()];
        let spec = install_spec(&packages);

        assert_eq!(spec.command(), "sudo");
        let args: Vec<&str> = spec.args().iter().map(|s| s.as_str()).collect();
        assert_eq!(args, vec!["pkg", "install", "-y", "git"]);
    }

    #[test]
    fn install_spec_handles_multiple_packages() {
        let packages = vec!["vim".into(), "curl".into(), "wget".into()];
        let spec = install_spec(&packages);

        let args: Vec<&str> = spec.args().iter().map(|s| s.as_str()).collect();
        assert_eq!(args, vec!["pkg", "install", "-y", "vim", "curl", "wget"]);
    }

    #[test]
    fn install_spec_handles_empty_packages() {
        let packages: Vec<String> = vec![];
        let spec = install_spec(&packages);

        assert_eq!(spec.command(), "sudo");
        let args: Vec<&str> = spec.args().iter().map(|s| s.as_str()).collect();
        assert_eq!(args, vec!["pkg", "install", "-y"]);
    }

    #[test]
    fn uninstall_spec_generates_correct_command() {
        let packages = vec!["git".into()];
        let spec = uninstall_spec(&packages);

        assert_eq!(spec.command(), "sudo");
        let args: Vec<&str> = spec.args().iter().map(|s| s.as_str()).collect();
        assert_eq!(args, vec!["pkg", "delete", "-y", "git"]);
    }

    #[test]
    fn search_spec_non_exact() {
        let spec = search_spec("ripgrep", false).unwrap();
        assert_eq!(spec.command(), "pkg");
        let args: Vec<&str> = spec.args().iter().map(|s| s.as_str()).collect();
        assert_eq!(args, vec!["search", "ripgrep"]);
    }

    #[test]
    fn search_spec_exact_uses_dash_e() {
        let spec = search_spec("git", true).unwrap();
        let args: Vec<&str> = spec.args().iter().map(|s| s.as_str()).collect();
        assert_eq!(args, vec!["search", "-e", "git"]);
    }

    #[test]
    fn upgrade_spec_handles_empty_packages() {
        let packages: Vec<String> = vec![];
        let spec = upgrade_spec(&packages);

        assert_eq!(spec.command(), "sudo");
        let args: Vec<&str> = spec.args().iter().map(|s| s.as_str()).collect();
        assert_eq!(args, vec!["pkg", "upgrade", "-y"]);
    }
}
