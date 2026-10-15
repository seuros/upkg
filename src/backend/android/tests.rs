use super::*;
use crate::backend::test_support::assert_spec;
use rstest::rstest;
use std::assert_matches;

#[rstest]
#[case(AndroidManager::Pkg, "pkg")]
#[case(AndroidManager::Apt, "apt")]
fn manager_name(#[case] manager: AndroidManager, #[case] expected: &str) {
    assert_eq!(manager.name(), expected);
}

#[rstest]
#[case(AndroidManager::Pkg, vec!["git".into()], "pkg", vec!["install", "-y", "git"])]
#[case(AndroidManager::Apt, vec!["curl".into()], "apt", vec!["install", "-y", "curl"])]
fn install_spec_generates_correct_commands(
    #[case] manager: AndroidManager,
    #[case] packages: Vec<String>,
    #[case] expected_command: &str,
    #[case] expected_args: Vec<&str>,
) {
    assert_spec(
        &manager.install_spec(&packages),
        expected_command,
        &expected_args,
    );
}

#[test]
fn install_spec_handles_multiple_packages() {
    let packages = vec!["vim".into(), "wget".into(), "git".into()];
    let spec = AndroidManager::Pkg.install_spec(&packages);
    assert_eq!(spec.args(), ["install", "-y", "vim", "wget", "git"]);
}

#[rstest]
#[case(AndroidManager::Pkg, vec!["git".into()], "pkg", vec!["uninstall", "-y", "git"])]
#[case(AndroidManager::Apt, vec!["curl".into()], "apt", vec!["remove", "-y", "curl"])]
fn uninstall_spec_generates_correct_commands(
    #[case] manager: AndroidManager,
    #[case] packages: Vec<String>,
    #[case] expected_command: &str,
    #[case] expected_args: Vec<&str>,
) {
    assert_spec(
        &manager.uninstall_spec(&packages),
        expected_command,
        &expected_args,
    );
}

#[rstest]
#[case(AndroidManager::Pkg, "pkg", vec!["search", "ripgrep"])]
#[case(AndroidManager::Apt, "apt", vec!["search", "ripgrep"])]
fn search_spec_non_exact(
    #[case] manager: AndroidManager,
    #[case] expected_command: &str,
    #[case] expected_args: Vec<&str>,
) {
    let spec = manager.search_spec("ripgrep", false).unwrap();
    assert_spec(&spec, expected_command, &expected_args);
}

#[rstest]
#[case(AndroidManager::Pkg)]
#[case(AndroidManager::Apt)]
fn search_spec_rejects_exact(#[case] manager: AndroidManager) {
    let err = manager.search_spec("git", true).expect_err("should reject");
    assert_matches!(err, UpkgError::Unsupported(_));
}

#[test]
fn upgrade_spec_handles_empty_packages() {
    let spec = AndroidManager::Pkg.upgrade_spec(&[]);
    assert_eq!(spec.args(), ["upgrade", "-y"]);
}
