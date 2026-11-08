use super::*;

#[cfg(unix)]
#[test]
fn system_runner_captures_both_streams_and_the_exit_code() {
    let spec = CommandSpec::new(
        "sh",
        vec!["-c".into(), "echo out; echo err >&2; exit 3".into()],
    );
    let execution = SystemRunner::new().run(&spec).unwrap();
    assert_eq!(execution.code, Some(3));
    assert_eq!(execution.stdout, "out\n");
    assert_eq!(execution.stderr, "err\n");
    assert!(!execution.success());
}

#[cfg(unix)]
#[test]
fn system_runner_passes_environment_and_closes_stdin() {
    let spec = CommandSpec::new("sh", vec!["-c".into(), "printf %s \"$PAGER\"; cat".into()])
        .env("PAGER", "cat");
    let execution = SystemRunner::new().run(&spec).unwrap();
    assert!(execution.success(), "{execution:?}");
    assert_eq!(execution.stdout, "cat");
}

#[test]
fn missing_program_is_an_error() {
    let spec = CommandSpec::new("upkg-no-such-program-12345", Vec::<String>::new());
    assert!(SystemRunner::new().run(&spec).is_err());
}

#[test]
fn drain_copies_what_it_captures() {
    let mut echoed = Vec::new();
    let captured = drain(Some(&b"hello"[..]), Some(&mut echoed)).unwrap();
    assert_eq!(captured, b"hello");
    assert_eq!(echoed, b"hello");
}
