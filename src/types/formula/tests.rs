use super::formula_token;

#[test]
fn formula_token_keeps_core_formula_name() {
    assert_eq!(formula_token("wget"), "wget");
}

#[test]
fn formula_token_extracts_tap_formula_name() {
    assert_eq!(formula_token("hashicorp/tap/terraform"), "terraform");
}

#[test]
fn formula_token_handles_empty_name_explicitly() {
    assert_eq!(formula_token(""), "");
}

#[test]
fn formula_token_ignores_trailing_separator() {
    assert_eq!(formula_token("hashicorp/tap/terraform/"), "terraform");
}

#[test]
fn formula_token_handles_only_separators() {
    assert_eq!(formula_token("///"), "");
}
