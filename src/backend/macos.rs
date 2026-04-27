use crate::backend::CommandSpec;

pub fn name() -> &'static str {
    "upkg"
}

pub fn install_spec(packages: &[String]) -> CommandSpec {
    let mut args = vec!["install".to_string()];
    args.extend(packages.iter().cloned());
    CommandSpec::new("upkg", args)
}

pub fn uninstall_spec(packages: &[String]) -> CommandSpec {
    let mut args = vec!["uninstall".to_string()];
    args.extend(packages.iter().cloned());
    CommandSpec::new("upkg", args)
}

pub fn upgrade_spec(packages: &[String]) -> CommandSpec {
    let mut args = vec!["upgrade".to_string()];
    args.extend(packages.iter().cloned());
    CommandSpec::new("upkg", args)
}
