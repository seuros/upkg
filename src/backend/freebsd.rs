use crate::backend::CommandSpec;

pub fn install_spec(packages: &[String]) -> CommandSpec {
    let mut args = vec!["pkg".to_string(), "install".to_string(), "-y".to_string()];
    args.extend(packages.iter().cloned());
    CommandSpec::new("sudo", args)
}

pub fn uninstall_spec(packages: &[String]) -> CommandSpec {
    let mut args = vec!["pkg".to_string(), "delete".to_string(), "-y".to_string()];
    args.extend(packages.iter().cloned());
    CommandSpec::new("sudo", args)
}

pub fn upgrade_spec(packages: &[String]) -> CommandSpec {
    let mut args = vec!["pkg".to_string(), "upgrade".to_string(), "-y".to_string()];
    args.extend(packages.iter().cloned());
    CommandSpec::new("sudo", args)
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
    fn upgrade_spec_handles_empty_packages() {
        let packages: Vec<String> = vec![];
        let spec = upgrade_spec(&packages);

        assert_eq!(spec.command(), "sudo");
        let args: Vec<&str> = spec.args().iter().map(|s| s.as_str()).collect();
        assert_eq!(args, vec!["pkg", "upgrade", "-y"]);
    }
}
