use std::process::Command;

#[test]
fn help_uses_stdout_and_succeeds() {
    for args in [
        vec!["--help"],
        vec!["help"],
        vec!["install", "--help"],
        vec!["help", "install"],
        vec!["--self-upgrade", "--help"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_upkg"))
            .args(&args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{args:?}: {output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains("Usage: upkg"), "{text}");
        if args.contains(&"install") || args.contains(&"--self-upgrade") {
            assert!(text.contains("--dry-run"), "{text}");
        }
    }
}

#[test]
fn version_output_is_unchanged() {
    for flag in ["--version", "-V"] {
        let output = Command::new(env!("CARGO_BIN_EXE_upkg"))
            .arg(flag)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!("upkg {}\n", env!("CARGO_PKG_VERSION"))
        );
    }
}

#[test]
fn invalid_input_uses_stderr_and_fails() {
    for args in [
        vec![],
        vec!["install"],
        vec!["nonesuch"],
        vec!["list", "extra"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_upkg"))
            .args(&args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{args:?}: {output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
        let text = String::from_utf8(output.stderr).unwrap();
        if args.is_empty() {
            // usage-rs explains a missing required subcommand with root help.
            assert!(text.contains("Usage: upkg"), "{text}");
        } else {
            assert_eq!(text.matches("error:").count(), 1, "{text}");
        }
        assert!(text.contains("--help"), "{text}");
    }
}
