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
fn monterey_rejects_sonoma_bottle() {
    let formula = bottled_formula("legacy", &[("sonoma", "https://example.com/sonoma.tar.gz")]);
    assert_matches!(
        select_bottle_with_codename(&formula, Some("monterey")),
        Err(Error::UnsupportedBottle { .. })
    );
}

#[cfg(target_os = "macos")]
fn macos_tag(codename: &str) -> String {
    #[cfg(target_arch = "aarch64")]
    return format!("arm64_{codename}");
    #[cfg(target_arch = "x86_64")]
    return codename.to_string();
}

#[rstest::rstest]
#[case("sonoma", "sequoia")]
#[case("sonoma", "tahoe")]
#[case("sequoia", "tahoe")]
#[case("monterey", "ventura")]
#[case("mojave", "big_sur")]
#[cfg(target_os = "macos")]
fn rejects_newer_macos_bottles(#[case] host: &'static str, #[case] bottle: &str) {
    let tag = macos_tag(bottle);
    let formula = bottled_formula("freetype", &[(&tag, "https://example.com/freetype.tar.gz")]);
    assert_matches!(
        select_bottle_with_codename(&formula, Some(host)),
        Err(Error::UnsupportedBottle { name }) if name == "freetype"
    );
}

#[rstest::rstest]
#[case("sonoma")]
#[case("ventura")]
#[cfg(target_os = "macos")]
fn sonoma_selects_same_or_older_bottle(#[case] compatible: &str) {
    let tag = macos_tag(compatible);
    assert_selects_for_codename(
        &[
            (&macos_tag("sequoia"), "https://example.com/newer.tar.gz"),
            (&tag, "https://example.com/compatible.tar.gz"),
        ],
        "sonoma",
        &tag,
    );
}

#[test]
#[cfg(target_os = "macos")]
fn sonoma_prefers_universal_bottle_over_newer_macos() {
    assert_selects_for_codename(
        &[
            (&macos_tag("sequoia"), "https://example.com/newer.tar.gz"),
            ("all", "https://example.com/universal.tar.gz"),
        ],
        "sonoma",
        "all",
    );
}

#[rstest::rstest]
#[case(None)]
#[case(Some("unknown"))]
#[cfg(target_os = "macos")]
fn unknown_macos_does_not_guess_a_bottle(#[case] host: Option<&'static str>) {
    let formula = bottled_formula(
        "freetype",
        &[(&macos_tag("sequoia"), "https://example.com/freetype.tar.gz")],
    );
    assert_matches!(
        select_bottle_with_codename(&formula, host),
        Err(Error::UnsupportedBottle { .. })
    );
}
