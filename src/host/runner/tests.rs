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

/// A runner that only runs commands, as one driving a jail would start out.
struct CommandsOnly;

impl Runner for CommandsOnly {
    fn run(&self, _spec: &CommandSpec) -> io::Result<Execution> {
        Ok(Execution::default())
    }
}

#[test]
fn a_runner_that_only_runs_commands_knows_nothing_of_this_machine() {
    let runner = CommandsOnly;
    assert!(!runner.is_root());
    // `sh` is on this machine's PATH; the runner doesn't say so.
    assert!(!runner.has_command("sh"));
    assert!(runner.filesystem().is_none());
}

#[test]
fn system_runner_reads_this_machines_files() {
    let runner = SystemRunner::new();
    let files = runner.filesystem().expect("the running system has files");
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    assert!(files.exists(&manifest));
    assert!(
        files
            .read_to_string(&manifest)
            .unwrap()
            .contains("name = \"upkg\"")
    );
    assert!(files.modified(&manifest).is_ok());
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    assert!(
        files
            .read_dir(&src)
            .unwrap()
            .contains(&"lib.rs".to_string())
    );
}
