use super::*;

fn spec(program: &str, args: &[&str]) -> CommandSpec {
    CommandSpec::new(
        program,
        args.iter().map(ToString::to_string).collect::<Vec<_>>(),
    )
}

#[test]
fn render_formats_command() {
    assert_eq!(
        spec("git", &["status", "--short"]).render(),
        "git status --short"
    );
    assert_eq!(spec("pwd", &[]).render(), "pwd");
}

#[test]
fn escalate_drops_sudo_for_root() {
    let root = spec("sudo", &["apt", "install", "git"]).escalate(Privilege::AlreadyRoot);
    assert_eq!(root.program(), "apt");
    assert_eq!(root.args(), ["install", "git"]);
}

#[test]
fn escalate_swaps_sudo_for_doas() {
    let doas = spec("sudo", &["apk", "add", "git"]).escalate(Privilege::Doas);
    assert_eq!(doas.render(), "doas apk add git");
}

#[test]
fn escalate_keeps_sudo_and_unprivileged_commands() {
    let sudo = spec("sudo", &["pkg", "install"]).escalate(Privilege::Sudo);
    assert_eq!(sudo.render(), "sudo pkg install");

    let plain = spec("apt", &["search", "git"]).escalate(Privilege::AlreadyRoot);
    assert_eq!(plain.render(), "apt search git");
}

#[test]
fn sudo_carries_environment_on_its_command_line() {
    let spec = spec("sudo", &["apt-get", "full-upgrade"]).env("DEBIAN_FRONTEND", "noninteractive");
    let (program, args) = spec.argv();
    assert_eq!(program, "sudo");
    assert_eq!(
        args,
        ["DEBIAN_FRONTEND=noninteractive", "apt-get", "full-upgrade"]
    );
    assert_eq!(spec.process_env(), []);
    assert_eq!(
        spec.render(),
        "sudo DEBIAN_FRONTEND=noninteractive apt-get full-upgrade"
    );
}

#[test]
fn doas_carries_environment_through_env() {
    let spec = spec("sudo", &["freebsd-update", "install"])
        .env("PAGER", "cat")
        .escalate(Privilege::Doas);
    assert_eq!(spec.render(), "doas env PAGER=cat freebsd-update install");
    assert_eq!(spec.process_env(), []);
}

#[test]
fn root_sets_environment_on_the_process() {
    let spec = spec("sudo", &["freebsd-update", "install"])
        .env("PAGER", "cat")
        .escalate(Privilege::AlreadyRoot);
    let (program, args) = spec.argv();
    assert_eq!(program, "freebsd-update");
    assert_eq!(args, ["install"]);
    assert_eq!(spec.process_env(), [("PAGER".into(), "cat".into())]);
    assert_eq!(spec.render(), "PAGER=cat freebsd-update install");
}

#[test]
fn explicit_privileges_resolve_to_themselves() {
    for privilege in [Privilege::AlreadyRoot, Privilege::Sudo, Privilege::Doas] {
        assert_eq!(privilege.resolve(), privilege);
    }
}

#[test]
fn arguments_append() {
    let spec = spec("sudo", &["pacman", "-Rns"]).arguments(["foo".to_string()]);
    assert_eq!(spec.args(), ["pacman", "-Rns", "foo"]);
}

#[cfg(unix)]
#[test]
fn command_exists_finds_sh_and_rejects_nonsense() {
    assert!(command_exists("sh"));
    assert!(!command_exists(
        "this_command_definitely_does_not_exist_12345"
    ));
}
