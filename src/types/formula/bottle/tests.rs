use super::*;
use crate::formula::types::{Bottle, BottleFile, BottleStable, KegOnly, Versions};
use std::assert_matches;
use std::collections::BTreeMap;

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
    let mut files = BTreeMap::new();
    files.insert(
        "all".to_string(),
        BottleFile {
            url: "https://ghcr.io/v2/homebrew/core/ca-certificates/blobs/sha256:abc123".to_string(),
            sha256: "abc123".to_string(),
        },
    );

    let formula = Formula {
        name: "ca-certificates".to_string(),
        versions: Versions {
            stable: "2024-01-01".to_string(),
        },
        dependencies: Vec::new(),
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
    };

    let selected = select_bottle(&formula).unwrap();
    assert_eq!(selected.tag, "all");
    assert!(selected.url.contains("ca-certificates"));
}

#[test]
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn errors_when_no_arm64_bottle() {
    let mut files = BTreeMap::new();
    files.insert(
        "sonoma".to_string(),
        BottleFile {
            url: "https://example.com/legacy.tar.gz".to_string(),
            sha256: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".to_string(),
        },
    );

    let formula = Formula {
        name: "legacy".to_string(),
        versions: Versions {
            stable: "0.1.0".to_string(),
        },
        dependencies: Vec::new(),
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
    };

    let err = select_bottle(&formula).unwrap_err();
    assert_matches!(
        err,
        Error::UnsupportedBottle { name } if name == "legacy"
    );
}

#[test]
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
fn errors_when_no_x86_64_bottle() {
    let mut files = BTreeMap::new();
    files.insert(
        "arm64_sonoma".to_string(),
        BottleFile {
            url: "https://example.com/legacy.tar.gz".to_string(),
            sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        },
    );

    let formula = Formula {
        name: "legacy".to_string(),
        versions: Versions {
            stable: "0.1.0".to_string(),
        },
        dependencies: Vec::new(),
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
    };

    let err = select_bottle(&formula).unwrap_err();
    assert_matches!(
        err,
        Error::UnsupportedBottle { name } if name == "legacy"
    );
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
    let mut files = BTreeMap::new();
    files.insert(
        "arm64_tahoe".to_string(),
        BottleFile {
            url: "https://example.com/tahoe.tar.gz".to_string(),
            sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        },
    );
    files.insert(
        "arm64_sequoia".to_string(),
        BottleFile {
            url: "https://example.com/sequoia.tar.gz".to_string(),
            sha256: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
        },
    );

    let formula = Formula {
        name: "current".to_string(),
        versions: Versions {
            stable: "1.0.0".to_string(),
        },
        dependencies: Vec::new(),
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
    };

    let selected = select_bottle_with_codename(&formula, Some("sequoia")).unwrap();
    assert_eq!(selected.tag, "arm64_sequoia");
}

#[test]
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
fn monterey_falls_back_to_sonoma_bottle() {
    let mut files = BTreeMap::new();
    files.insert(
        "sonoma".to_string(),
        BottleFile {
            url: "https://example.com/sonoma.tar.gz".to_string(),
            sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        },
    );

    let formula = Formula {
        name: "protobuf".to_string(),
        versions: Versions {
            stable: "35.0".to_string(),
        },
        dependencies: Vec::new(),
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
    };

    let selected = select_bottle_with_codename(&formula, Some("monterey")).unwrap();
    assert_eq!(selected.tag, "sonoma");
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
    let mut files = BTreeMap::new();
    files.insert(
        "big_sur".to_string(),
        BottleFile {
            url: "https://example.com/big-sur.tar.gz".to_string(),
            sha256: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".to_string(),
        },
    );

    let formula = Formula {
        name: "modern-only".to_string(),
        versions: Versions {
            stable: "1.0.0".to_string(),
        },
        dependencies: Vec::new(),
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
    };

    // With newer-bottle fallback, mojave user gets the closest newer bottle (big_sur)
    let selected = select_bottle_with_codename(&formula, Some("mojave")).unwrap();
    assert_eq!(selected.tag, "big_sur");
}
