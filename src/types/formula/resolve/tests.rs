use super::*;
use crate::formula::test_support::{names, test_formula};
use std::assert_matches;
use std::collections::BTreeMap;

fn formula(name: &str, deps: &[&str]) -> Formula {
    Formula {
        dependencies: names(deps),
        ..test_formula(name)
    }
}

#[test]
fn resolves_transitive_closure_in_stable_order() {
    let mut formulas = BTreeMap::new();
    formulas.insert("foo".to_string(), formula("foo", &["baz", "bar"]));
    formulas.insert("bar".to_string(), formula("bar", &["qux"]));
    formulas.insert("baz".to_string(), formula("baz", &["qux"]));
    formulas.insert("qux".to_string(), formula("qux", &[]));

    let order = resolve_closure(&["foo".to_string()], &formulas).unwrap();
    assert_eq!(order, vec!["qux", "bar", "baz", "foo"]);
}

#[test]
fn resolves_multiple_roots_with_shared_deps() {
    let mut formulas = BTreeMap::new();
    formulas.insert("a".to_string(), formula("a", &["shared"]));
    formulas.insert("b".to_string(), formula("b", &["shared"]));
    formulas.insert("shared".to_string(), formula("shared", &[]));

    let order = resolve_closure(&["a".to_string(), "b".to_string()], &formulas).unwrap();
    assert_eq!(order, vec!["shared", "a", "b"]);
}

#[test]
fn detects_cycles() {
    let mut formulas = BTreeMap::new();
    formulas.insert("alpha".to_string(), formula("alpha", &["beta"]));
    formulas.insert("beta".to_string(), formula("beta", &["gamma"]));
    formulas.insert("gamma".to_string(), formula("gamma", &["alpha"]));

    let err = resolve_closure(&["alpha".to_string()], &formulas).unwrap_err();
    assert_matches!(err, Error::DependencyCycle { .. });
}

#[test]
fn skips_missing_dependencies() {
    let mut formulas = BTreeMap::new();
    formulas.insert("git".to_string(), formula("git", &["gettext", "libiconv"]));
    formulas.insert("gettext".to_string(), formula("gettext", &[]));

    let order = resolve_closure(&["git".to_string()], &formulas).unwrap();
    assert_eq!(order, vec!["gettext", "git"]);
}
