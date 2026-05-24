use crate::{Error, Formula};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedBottle {
    pub tag: String,
    pub url: String,
    pub sha256: String,
}

const MACOS_CODENAMES_NEWEST_FIRST: &[&str] = &[
    "tahoe",
    "sequoia",
    "sonoma",
    "ventura",
    "monterey",
    "big_sur",
    "catalina",
    "mojave",
    "high_sierra",
];

#[cfg(target_os = "macos")]
fn current_macos_codename() -> Option<&'static str> {
    let output = std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok()?;
    let version = String::from_utf8_lossy(&output.stdout);
    codename_for_product_version(version.trim())
}

#[cfg(target_os = "macos")]
fn codename_for_product_version(version: &str) -> Option<&'static str> {
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().and_then(|part| part.parse().ok());
    codename_for_version_parts(major, minor)
}

#[cfg(target_os = "macos")]
fn codename_for_version_parts(major: u32, minor: Option<u32>) -> Option<&'static str> {
    match (major, minor) {
        (10, Some(15)) => Some("catalina"),
        (10, Some(14)) => Some("mojave"),
        (10, Some(13)) => Some("high_sierra"),
        (major, _) => match major {
            26 => Some("tahoe"),
            15 => Some("sequoia"),
            14 => Some("sonoma"),
            13 => Some("ventura"),
            12 => Some("monterey"),
            11 => Some("big_sur"),
            _ => None,
        },
    }
}

pub fn compatible_codenames(current_codename: Option<&'static str>) -> Vec<&'static str> {
    let Some(codename) = current_codename else {
        return Vec::new();
    };

    let Some(pos) = MACOS_CODENAMES_NEWEST_FIRST
        .iter()
        .position(|&tag| tag == codename)
    else {
        return Vec::new();
    };

    MACOS_CODENAMES_NEWEST_FIRST[pos..].to_vec()
}

/// Returns newer codenames ordered from closest-newer to newest.
/// Used as fallback when no same-or-older bottle exists.
fn newer_codenames(current_codename: Option<&'static str>) -> Vec<&'static str> {
    let Some(codename) = current_codename else {
        return Vec::new();
    };

    let Some(pos) = MACOS_CODENAMES_NEWEST_FIRST
        .iter()
        .position(|&tag| tag == codename)
    else {
        return Vec::new();
    };

    if pos == 0 {
        return Vec::new();
    }

    let mut newer: Vec<&str> = MACOS_CODENAMES_NEWEST_FIRST[..pos].to_vec();
    newer.reverse(); // closest-newer first
    newer
}

pub fn select_bottle(formula: &Formula) -> Result<SelectedBottle, Error> {
    #[cfg(target_os = "macos")]
    let macos_codename = current_macos_codename();
    #[cfg(not(target_os = "macos"))]
    let macos_codename: Option<&'static str> = None;

    select_bottle_with_codename(formula, macos_codename)
}

pub fn current_platform_bottle_candidates() -> Vec<String> {
    platform_bottle_candidates()
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn platform_bottle_candidates() -> Vec<String> {
    let Some(codename) = current_macos_codename() else {
        return Vec::new();
    };
    compatible_codenames(Some(codename))
        .into_iter()
        .map(|codename| format!("arm64_{codename}"))
        .collect()
}

#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
fn platform_bottle_candidates() -> Vec<String> {
    let Some(codename) = current_macos_codename() else {
        return Vec::new();
    };
    compatible_codenames(Some(codename))
        .into_iter()
        .map(ToString::to_string)
        .collect()
}

#[cfg(not(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "macos", target_arch = "x86_64"),
)))]
fn platform_bottle_candidates() -> Vec<String> {
    Vec::new()
}

pub fn current_platform_bottle_tag() -> Option<String> {
    current_platform_bottle_candidates().into_iter().next()
}

fn select_bottle_with_codename(
    formula: &Formula,
    macos_codename: Option<&'static str>,
) -> Result<SelectedBottle, Error> {
    let _ = &macos_codename;

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        let tags: Vec<String> = compatible_codenames(macos_codename)
            .iter()
            .map(|codename| format!("arm64_{codename}"))
            .collect();

        for tag in &tags {
            if let Some(file) = formula.bottle.stable.files.get(tag.as_str()) {
                return Ok(SelectedBottle {
                    tag: tag.clone(),
                    url: file.url.clone(),
                    sha256: file.sha256.clone(),
                });
            }
        }
    }

    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        for tag in compatible_codenames(macos_codename) {
            if let Some(file) = formula.bottle.stable.files.get(tag) {
                return Ok(SelectedBottle {
                    tag: tag.to_string(),
                    url: file.url.clone(),
                    sha256: file.sha256.clone(),
                });
            }
        }
    }

    if let Some(file) = formula.bottle.stable.files.get("all") {
        return Ok(SelectedBottle {
            tag: "all".to_string(),
            url: file.url.clone(),
            sha256: file.sha256.clone(),
        });
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        let compatible = compatible_codenames(macos_codename);
        for (tag, file) in &formula.bottle.stable.files {
            if let Some(codename) = tag.strip_prefix("arm64_")
                && compatible.contains(&codename)
            {
                return Ok(SelectedBottle {
                    tag: tag.clone(),
                    url: file.url.clone(),
                    sha256: file.sha256.clone(),
                });
            }
        }
    }

    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        let compatible = compatible_codenames(macos_codename);
        for (tag, file) in &formula.bottle.stable.files {
            if !tag.starts_with("arm64_") && compatible.contains(&tag.as_str()) {
                return Ok(SelectedBottle {
                    tag: tag.clone(),
                    url: file.url.clone(),
                    sha256: file.sha256.clone(),
                });
            }
        }
    }

    // Fallback: try closest newer bottle when no same-or-older bottle exists.
    // Homebrew bottles built on newer macOS generally work on older versions.
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        for codename in newer_codenames(macos_codename) {
            let tag = format!("arm64_{codename}");
            if let Some(file) = formula.bottle.stable.files.get(tag.as_str()) {
                return Ok(SelectedBottle {
                    tag,
                    url: file.url.clone(),
                    sha256: file.sha256.clone(),
                });
            }
        }
    }

    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        for codename in newer_codenames(macos_codename) {
            if let Some(file) = formula.bottle.stable.files.get(codename) {
                return Ok(SelectedBottle {
                    tag: codename.to_string(),
                    url: file.url.clone(),
                    sha256: file.sha256.clone(),
                });
            }
        }
    }

    Err(Error::UnsupportedBottle {
        name: formula.name.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formula::types::{Bottle, BottleFile, BottleStable, KegOnly, Versions};
    use std::collections::BTreeMap;

    #[test]
    fn selects_platform_bottle() {
        let fixture = include_str!("../../fixtures/formula_foo.json");
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
            assert!(matches!(
                select_bottle(&formula),
                Err(Error::UnsupportedBottle { name }) if name == formula.name
            ));
        }
    }

    #[test]
    fn selects_all_bottle_for_universal_packages() {
        let mut files = BTreeMap::new();
        files.insert(
            "all".to_string(),
            BottleFile {
                url: "https://ghcr.io/v2/homebrew/core/ca-certificates/blobs/sha256:abc123"
                    .to_string(),
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
                sha256: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                    .to_string(),
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
        assert!(matches!(
            err,
            Error::UnsupportedBottle { name } if name == "legacy"
        ));
    }

    #[test]
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    fn errors_when_no_x86_64_bottle() {
        let mut files = BTreeMap::new();
        files.insert(
            "arm64_sonoma".to_string(),
            BottleFile {
                url: "https://example.com/legacy.tar.gz".to_string(),
                sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
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
        assert!(matches!(
            err,
            Error::UnsupportedBottle { name } if name == "legacy"
        ));
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
                sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
            },
        );
        files.insert(
            "arm64_sequoia".to_string(),
            BottleFile {
                url: "https://example.com/sequoia.tar.gz".to_string(),
                sha256: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                    .to_string(),
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
                sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
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
                sha256: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                    .to_string(),
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
}
