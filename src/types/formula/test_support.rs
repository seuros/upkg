//! Shared `Formula` fixtures for unit tests under `crate::types`.

use super::types::{Bottle, BottleFile, BottleStable, Formula, KegOnly, Versions};

/// Minimal formula at version `1.0.0`: no dependencies, no bottles, no source.
/// Override fields with struct-update syntax: `Formula { .., ..test_formula("x") }`.
pub(crate) fn test_formula(name: &str) -> Formula {
    Formula {
        name: name.to_string(),
        versions: Versions {
            stable: "1.0.0".to_string(),
        },
        dependencies: Vec::new(),
        bottle: bottle(&[]),
        revision: 0,
        keg_only: KegOnly::default(),
        build_dependencies: Vec::new(),
        urls: None,
        ruby_source_path: None,
        ruby_source_checksum: None,
        uses_from_macos: Vec::new(),
        requirements: Vec::new(),
        variations: None,
        post_install_steps: Vec::new(),
    }
}

/// Formula whose stable bottle set is exactly `files`, given as `(tag, url)`.
pub(crate) fn bottled_formula(name: &str, files: &[(&str, &str)]) -> Formula {
    Formula {
        bottle: bottle(files),
        ..test_formula(name)
    }
}

/// Stable bottle set from `(tag, url)` pairs; every file shares a dummy sha256.
pub(crate) fn bottle(files: &[(&str, &str)]) -> Bottle {
    let files = files
        .iter()
        .map(|&(tag, url)| {
            let file = BottleFile {
                url: url.to_string(),
                sha256: "deadbeef".repeat(8),
            };
            (tag.to_string(), file)
        })
        .collect();

    Bottle {
        stable: BottleStable { files, rebuild: 0 },
    }
}

/// Lists dependency names as the owned `Vec<String>` `Formula` stores.
pub(crate) fn names(deps: &[&str]) -> Vec<String> {
    deps.iter().map(|&dep| dep.to_string()).collect()
}
