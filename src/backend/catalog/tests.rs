use super::*;

#[test]
fn embedded_catalog_parses() {
    // Forces the OnceLock init; panics here if the shipped TOML is invalid.
    assert!(!catalog().is_empty());
}

#[test]
fn divergent_name_resolves_per_backend() {
    assert_eq!(resolve_one("dnf", "libpq-dev"), "libpq-devel");
    assert_eq!(resolve_one("pacman", "libpq-dev"), "postgresql-libs");
    // apt happens to match the canonical key; entry exists because OTHER
    // backends diverge, which is the whole point.
    assert_eq!(resolve_one("apt", "libpq-dev"), "libpq-dev");
}

#[test]
fn unknown_name_falls_through_verbatim() {
    assert_eq!(resolve_one("dnf", "ripgrep"), "ripgrep");
    assert_eq!(resolve_one("apt", "git"), "git");
}

#[test]
fn missing_backend_key_falls_through_verbatim() {
    // libpq-dev has no `freebsd` column → canonical name passes through.
    assert_eq!(resolve_one("freebsd", "libpq-dev"), "libpq-dev");
}

#[test]
fn resolve_preserves_order_and_count() {
    let input = vec!["git".to_string(), "libpq-dev".to_string()];
    assert_eq!(resolve("dnf", &input), vec!["git", "libpq-devel"]);
}

#[test]
fn no_identity_only_entries() {
    // Policy guard: every entry must diverge on at least one backend.
    // An entry whose every name equals the canonical key is a registry
    // entry, not an alias, and should never have been added.
    for (canonical, entry) in catalog() {
        let diverges = entry
            .iter()
            .filter(|(key, _)| key.as_str() != UPSTREAM_KEY)
            .any(|(_, name)| name != canonical);
        assert!(
            diverges,
            "catalog entry `{canonical}` has no divergent name; it does not belong in the catalog"
        );
    }
}

#[test]
fn every_entry_declares_upstream() {
    // The anti-typo-squat anchor: a reviewer must be able to verify identity.
    for (canonical, entry) in catalog() {
        assert!(
            entry.contains_key(UPSTREAM_KEY),
            "catalog entry `{canonical}` is missing an `upstream`"
        );
    }
}
