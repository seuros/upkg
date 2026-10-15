use super::*;
use crate::formula::test_support::bottled_formula;
use std::assert_matches;

/// A formula offering only a `tag` bottle is unsupported on this platform.
fn assert_unsupported_with_only(tag: &str) {
    let formula = bottled_formula("legacy", &[(tag, "https://example.com/legacy.tar.gz")]);

    let err = select_bottle(&formula).unwrap_err();
    assert_matches!(
        err,
        Error::UnsupportedBottle { name } if name == "legacy"
    );
}

/// On macOS `codename`, `files` resolve to the bottle tagged `expected`.
fn assert_selects_for_codename(files: &[(&str, &str)], codename: &'static str, expected: &str) {
    let formula = bottled_formula("current", files);

    let selected = select_bottle_with_codename(&formula, Some(codename)).unwrap();
    assert_eq!(
        selected.tag, expected,
        "codename: {codename}, files: {files:?}"
    );
}

#[test]
fn selects_platform_bottle() {
    let fixture = include_str!("../../../fixtures/formula_foo.json");
    let formula: Formula = serde_json::from_str(fixture).unwrap();

    let expected = current_platform_bottle_candidates()
        .into_iter()
        .find(|tag| formula.bottle.stable.files.contains_key(tag));

    if let Some(expected) = expected {
        let selected = select_bottle(&formula).unwrap();
        assert_eq!(selected.tag, expected);
    } else if formula.bottle.stable.files.contains_key("all") {
        let selected = select_bottle(&formula).unwrap();
        assert_eq!(selected.tag, "all");
    } else {
        assert_matches!(
            select_bottle(&formula),
            Err(Error::UnsupportedBottle { name }) if name == formula.name
        );
    }
}

#[test]
fn selects_all_bottle_for_universal_packages() {
    let formula = bottled_formula(
        "ca-certificates",
        &[(
            "all",
            "https://ghcr.io/v2/homebrew/core/ca-certificates/blobs/sha256:abc123",
        )],
    );

    let selected = select_bottle(&formula).unwrap();
    assert_eq!(selected.tag, "all");
    assert!(selected.url.contains("ca-certificates"));
}

#[test]
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn errors_when_no_arm64_bottle() {
    assert_unsupported_with_only("sonoma");
}

#[test]
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
fn errors_when_no_x86_64_bottle() {
    assert_unsupported_with_only("arm64_sonoma");
}

#[test]
fn compatible_codenames_on_sequoia_excludes_tahoe() {
    assert_eq!(
        compatible_codenames(Some("sequoia")),
        vec![
            "sequoia",
            "sonoma",
            "ventura",
            "monterey",
            "big_sur",
            "catalina",
            "mojave",
            "high_sierra"
        ]
    );
}

#[test]
fn compatible_codenames_on_tahoe_includes_tahoe() {
    assert_eq!(
        compatible_codenames(Some("tahoe")),
        vec![
            "tahoe",
            "sequoia",
            "sonoma",
            "ventura",
            "monterey",
            "big_sur",
            "catalina",
            "mojave",
            "high_sierra"
        ]
    );
}

#[test]
fn compatible_codenames_on_mojave_do_not_include_newer_releases() {
    assert_eq!(
        compatible_codenames(Some("mojave")),
        vec!["mojave", "high_sierra"]
    );
}

#[test]
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn sequoia_user_skips_tahoe_bottle() {
    assert_selects_for_codename(
        &[
            ("arm64_tahoe", "https://example.com/tahoe.tar.gz"),
            ("arm64_sequoia", "https://example.com/sequoia.tar.gz"),
        ],
        "sequoia",
        "arm64_sequoia",
    );
}

#[test]
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
fn monterey_falls_back_to_sonoma_bottle() {
    assert_selects_for_codename(
        &[("sonoma", "https://example.com/sonoma.tar.gz")],
        "monterey",
        "sonoma",
    );
}

#[test]
fn newer_codenames_returns_closest_first() {
    let newer = newer_codenames(Some("monterey"));
    assert_eq!(newer, vec!["ventura", "sonoma", "sequoia", "tahoe"]);
}

#[test]
fn newer_codenames_empty_for_newest() {
    let newer = newer_codenames(Some("tahoe"));
    assert!(newer.is_empty());
}

#[test]
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
fn mojave_falls_back_to_closest_newer_bottle() {
    // With newer-bottle fallback, mojave user gets the closest newer bottle (big_sur)
    assert_selects_for_codename(
        &[("big_sur", "https://example.com/big-sur.tar.gz")],
        "mojave",
        "big_sur",
    );
}
