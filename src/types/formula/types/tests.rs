use super::*;
use std::assert_matches;

#[test]
fn deserialize_formula_fixtures() {
    let fixtures = [
        include_str!("../../../fixtures/formula_foo.json"),
        include_str!("../../../fixtures/formula_bar.json"),
    ];

    for fixture in fixtures {
        let formula: Formula = serde_json::from_str(fixture).unwrap();
        assert!(!formula.name.is_empty());
        assert!(!formula.versions.stable.is_empty());
        assert!(!formula.bottle.stable.files.is_empty());
    }
}

#[test]
fn effective_version_without_revision() {
    let fixture = include_str!("../../../fixtures/formula_foo.json");
    let formula: Formula = serde_json::from_str(fixture).unwrap();

    assert_eq!(formula.revision, 0);
    assert_eq!(formula.effective_version(), "1.2.3");
}

#[test]
fn effective_version_with_revision() {
    let mut formula: Formula =
        serde_json::from_str(include_str!("../../../fixtures/formula_foo.json")).unwrap();
    formula.revision = 1;

    assert_eq!(formula.effective_version(), "1.2.3_1");
}

#[test]
fn effective_version_ignores_rebuild_for_dir_name() {
    let fixture = include_str!("../../../fixtures/formula_with_rebuild.json");
    let formula: Formula = serde_json::from_str(fixture).unwrap();

    assert_eq!(formula.bottle.stable.rebuild, 2);
    assert_eq!(formula.revision, 0);
    assert_eq!(formula.effective_version(), "1.0.0");
}

#[test]
fn revision_field_defaults_to_zero() {
    let fixture = include_str!("../../../fixtures/formula_foo.json");
    let formula: Formula = serde_json::from_str(fixture).unwrap();
    assert_eq!(formula.revision, 0);
}

#[test]
fn keg_only_defaults_to_no() {
    let fixture = include_str!("../../../fixtures/formula_foo.json");
    let formula: Formula = serde_json::from_str(fixture).unwrap();
    assert_eq!(formula.keg_only, KegOnly::No);
    assert!(!formula.is_keg_only());
}

#[test]
fn keg_only_deserializes_bool_true() {
    let json = r#"{
            "name": "libfoo",
            "versions": { "stable": "1.0" },
            "dependencies": [],
            "keg_only": true,
            "bottle": { "stable": { "files": {
                "arm64_sonoma": { "url": "https://x.com/a.tar.gz", "sha256": "aa" }
            }}}
        }"#;
    let formula: Formula = serde_json::from_str(json).unwrap();
    assert_eq!(formula.keg_only, KegOnly::Yes);
    assert!(formula.is_keg_only());
}

#[test]
fn keg_only_deserializes_string_reason() {
    let json = r#"{
            "name": "libpq",
            "versions": { "stable": "16.0" },
            "dependencies": [],
            "keg_only": "it conflicts with PostgreSQL",
            "bottle": { "stable": { "files": {
                "arm64_sonoma": { "url": "https://x.com/a.tar.gz", "sha256": "aa" }
            }}}
        }"#;
    let formula: Formula = serde_json::from_str(json).unwrap();
    assert_matches!(
        formula.keg_only,
        KegOnly::Reason(ref s) if s == "it conflicts with PostgreSQL"
    );
    assert!(formula.is_keg_only());
}

#[test]
fn versioned_formula_is_keg_only() {
    let json = r#"{
            "name": "postgresql@15",
            "versions": { "stable": "15.8" },
            "dependencies": [],
            "bottle": { "stable": { "files": {
                "arm64_sonoma": { "url": "https://x.com/a.tar.gz", "sha256": "aa" }
            }}}
        }"#;
    let formula: Formula = serde_json::from_str(json).unwrap();
    assert_eq!(formula.keg_only, KegOnly::No);
    assert!(formula.is_keg_only());
}
