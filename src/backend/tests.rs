use super::*;

#[test]
fn command_spec_new_creates_spec() {
    let spec = CommandSpec::new("ls", vec!["-la".to_string()]);
    assert_eq!(spec.command(), "ls");
    assert_eq!(spec.args(), &["-la"]);
}

#[test]
fn command_spec_render_formats_command() {
    let spec = CommandSpec::new("git", vec!["status".to_string(), "--short".to_string()]);
    assert_eq!(spec.render(), "git status --short");
}

#[cfg(unix)]
#[test]
fn escalate_drops_sudo_for_root() {
    let spec = CommandSpec::new("sudo", vec!["apt".into(), "install".into(), "git".into()]);
    let spec = spec.escalate(Escalation::Root);
    assert_eq!(spec.command(), "apt");
    assert_eq!(spec.args(), &["install", "git"]);
}

#[cfg(unix)]
#[test]
fn escalate_swaps_sudo_for_doas() {
    let spec = CommandSpec::new("sudo", vec!["apk".into(), "add".into(), "git".into()]);
    let spec = spec.escalate(Escalation::Doas);
    assert_eq!(spec.command(), "doas");
    assert_eq!(spec.args(), &["apk", "add", "git"]);
}

#[cfg(unix)]
#[test]
fn escalate_keeps_sudo_and_unprivileged_commands() {
    let spec = CommandSpec::new("sudo", vec!["pkg".into(), "install".into()]);
    assert_eq!(spec.escalate(Escalation::Sudo).render(), "sudo pkg install");

    let spec = CommandSpec::new("apt", vec!["search".into(), "git".into()]);
    assert_eq!(spec.escalate(Escalation::Root).render(), "apt search git");
}

#[test]
fn command_spec_render_handles_empty_args() {
    let spec = CommandSpec::new("pwd", Vec::<String>::new());
    assert_eq!(spec.render(), "pwd");
}

#[cfg(any(target_os = "android", target_os = "linux", target_os = "windows"))]
#[test]
fn command_exists_finds_common_command() {
    // 'sh' should exist on Linux/Android, 'cmd' on Windows
    #[cfg(unix)]
    let result = command_exists("sh");
    #[cfg(windows)]
    let result = command_exists("cmd");

    assert!(result);
}

#[cfg(any(target_os = "android", target_os = "linux", target_os = "windows"))]
#[test]
fn command_exists_rejects_nonexistent_command() {
    let result = command_exists("this_command_definitely_does_not_exist_12345");
    assert!(!result);
}
