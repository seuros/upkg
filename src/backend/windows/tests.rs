use super::*;
use crate::backend::test_support::assert_spec;
use rstest::rstest;

#[rstest]
#[case(WindowsManager::Winget, "winget")]
#[case(WindowsManager::Choco, "choco")]
fn manager_name(#[case] manager: WindowsManager, #[case] expected: &str) {
    assert_eq!(manager.name(), expected);
}

#[rstest]
#[case(WindowsManager::Winget, vec!["git".into()], "winget", vec!["install", "--silent", "--accept-source-agreements", "--accept-package-agreements", "git"])]
#[case(WindowsManager::Choco, vec!["curl".into()], "choco", vec!["install", "-y", "curl"])]
fn install_spec_generates_correct_commands(
    #[case] manager: WindowsManager,
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
    let packages = vec!["nodejs".into(), "python".into(), "git".into()];
    let spec = WindowsManager::Winget.install_spec(&packages);
    assert_eq!(
        spec.args(),
        [
            "install",
            "--silent",
            "--accept-source-agreements",
            "--accept-package-agreements",
            "nodejs",
            "python",
            "git"
        ]
    );
}

#[rstest]
#[case(WindowsManager::Winget, vec!["git".into()], "winget", vec!["uninstall", "--silent", "--accept-source-agreements", "git"])]
#[case(WindowsManager::Choco, vec!["curl".into()], "choco", vec!["uninstall", "-y", "curl"])]
fn uninstall_spec_generates_correct_commands(
    #[case] manager: WindowsManager,
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
#[case(WindowsManager::Winget, false, "winget", vec!["search", "ripgrep"])]
#[case(WindowsManager::Winget, true, "winget", vec!["search", "-e", "ripgrep"])]
#[case(WindowsManager::Choco, false, "choco", vec!["search", "ripgrep"])]
#[case(WindowsManager::Choco, true, "choco", vec!["search", "ripgrep", "-e"])]
fn search_spec_generates_correct_commands(
    #[case] manager: WindowsManager,
    #[case] exact: bool,
    #[case] expected_command: &str,
    #[case] expected_args: Vec<&str>,
) {
    let spec = manager.search_spec("ripgrep", exact).unwrap();
    assert_spec(&spec, expected_command, &expected_args);
}

#[rstest]
#[case(WindowsManager::Winget, "winget", vec!["list"])]
#[case(WindowsManager::Choco, "choco", vec!["list", "--local-only"])]
fn list_spec_generates_correct_commands(
    #[case] manager: WindowsManager,
    #[case] expected_command: &str,
    #[case] expected_args: Vec<&str>,
) {
    assert_spec(&manager.list_spec(), expected_command, &expected_args);
}
