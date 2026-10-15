use super::*;
use crate::backend::test_support::{assert_exact_search_uses_dash_e, assert_spec};

#[test]
fn install_spec_generates_correct_command() {
    let spec = install_spec(&["git".into()]);
    assert_spec(&spec, RVN_PATH, &["install", "-y", "git"]);
}

#[test]
fn install_spec_handles_multiple_packages() {
    let spec = install_spec(&["vim".into(), "curl".into(), "wget".into()]);
    assert_eq!(spec.args(), ["install", "-y", "vim", "curl", "wget"]);
}

#[test]
fn install_spec_handles_empty_packages() {
    assert_spec(&install_spec(&[]), RVN_PATH, &["install", "-y"]);
}

#[test]
fn uninstall_spec_generates_correct_command() {
    let spec = uninstall_spec(&["git".into()]);
    assert_spec(&spec, RVN_PATH, &["remove", "-y", "git"]);
}

#[test]
fn search_spec_non_exact() {
    let spec = search_spec("ripgrep", false).unwrap();
    assert_spec(&spec, RVN_PATH, &["search", "ripgrep"]);
}

#[test]
fn search_spec_exact() {
    assert_exact_search_uses_dash_e(search_spec);
}

#[test]
fn upgrade_spec_handles_empty_packages() {
    assert_spec(&upgrade_spec(&[]), RVN_PATH, &["upgrade", "-y"]);
}
