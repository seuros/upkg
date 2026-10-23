use super::*;
use test_support::{assert_answers, assert_spec};

#[test]
fn command_spec_new_creates_spec() {
    let spec = CommandSpec::new("ls", vec!["-la".to_string()]);
    assert_spec(&spec, "ls", &["-la"]);
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
    assert_spec(&spec.escalate(Escalation::Root), "apt", &["install", "git"]);
}

#[cfg(unix)]
#[test]
fn escalate_swaps_sudo_for_doas() {
    let spec = CommandSpec::new("sudo", vec!["apk".into(), "add".into(), "git".into()]);
    assert_spec(
        &spec.escalate(Escalation::Doas),
        "doas",
        &["apk", "add", "git"],
    );
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

#[test]
fn exit_status_answer_is_the_exit_status() {
    assert_answers(&[
        (Answer::ExitStatus, true, "", true),
        (Answer::ExitStatus, false, "", false),
        // pkg/apk echo the name when installed; only the status counts.
        (Answer::ExitStatus, false, "git\n", false),
    ]);
}
