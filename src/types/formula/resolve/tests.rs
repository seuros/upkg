use super::*;
use crate::formula::types::{Bottle, BottleFile, BottleStable, KegOnly, Versions};
use std::assert_matches;
use std::collections::BTreeMap;

fn formula(name: &str, deps: &[&str]) -> Formula {
    let mut files = BTreeMap::new();
    files.insert(
        "arm64_sonoma".to_string(),
        BottleFile {
            url: format!("https://example.com/{name}.tar.gz"),
            sha256: "deadbeef".repeat(8),
        },
    );

    Formula {
        name: name.to_string(),
        versions: Versions {
            stable: "1.0.0".to_string(),
        },
        dependencies: deps.iter().map(|dep| dep.to_string()).collect(),
        bottle: Bottle {
            stable: BottleStable { files, rebuild: 0 },
        },
        revision: 0,
        keg_only: KegOnly::default(),
        build_dependencies: Vec::new(),
        urls: None,
        ruby_source_path: None,
        ruby_source_checksum: None,
        uses_from_macos: Vec::new(),
        requirements: Vec::new(),
        variations: None,
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
