use super::*;
use std::assert_matches;

#[test]
fn normalize_core_tap_formula() {
    assert_eq!(
        normalize_formula_name("homebrew/core/wget").unwrap(),
        "wget".to_string()
    );
}

#[test]
fn normalize_external_tap_formula_keeps_full_name() {
    assert_eq!(
        normalize_formula_name("hashicorp/tap/terraform").unwrap(),
        "hashicorp/tap/terraform".to_string()
    );
}

#[test]
fn normalize_homebrew_cask_prefixes_token() {
    assert_eq!(
        normalize_formula_name("homebrew/cask/docker-desktop").unwrap(),
        "cask:docker-desktop".to_string()
    );
}

#[test]
fn normalize_app_name_prefixes_plain_token() {
    assert_eq!(normalize_app_name("ghostty").unwrap(), "cask:ghostty");
}

#[test]
fn normalize_app_name_rejects_non_cask_tap() {
    let err = normalize_app_name("hashicorp/tap/terraform").unwrap_err();
    assert_matches!(err, Error::InvalidArgument { .. });
}
