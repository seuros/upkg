use super::*;

fn checked(requested: &str, native: &str, installed: bool) -> Checked {
    Checked {
        requested: requested.to_string(),
        native: native.to_string(),
        installed,
    }
}

#[test]
fn line_is_tab_separated_state_requested_native() {
    const CASES: &[(&str, &str, bool, &str)] = &[
        ("git", "git", true, "installed\tgit\tgit"),
        (
            "libpq-dev",
            "postgresql-libs",
            false,
            "missing\tlibpq-dev\tpostgresql-libs",
        ),
    ];
    for &(requested, native, installed, expected) in CASES {
        assert_eq!(checked(requested, native, installed).line(), expected);
    }
}

#[test]
fn exit_code_fails_when_anything_is_missing() {
    let all = [checked("git", "git", true), checked("curl", "curl", true)];
    assert_eq!(exit_code(&all), ExitCode::SUCCESS);

    let some = [
        checked("git", "git", true),
        checked("llvm19", "llvm19", false),
    ];
    assert_eq!(exit_code(&some), ExitCode::from(1));
}
