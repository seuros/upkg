use super::*;
use test_support::{assert_answers, assert_spec};

#[test]
fn command_spec_new_creates_spec() {
    let spec = CommandSpec::new("ls", vec!["-la".to_string()]);
    assert_spec(&spec, "ls", &["-la"]);
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
