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

/// Every backend key a host can resolve with; anything else in an entry is
/// a typo that would never match.
const BACKEND_KEYS: &[&str] = &[
    "apt", "dnf", "yum", "pacman", "zypper", "apk", "opkg", "freebsd", "rvn", "termux", "brew",
    "winget", "choco",
];

#[test]
fn every_key_is_a_known_backend() {
    for (canonical, entry) in catalog() {
        for key in entry.keys().filter(|key| key.as_str() != UPSTREAM_KEY) {
            assert!(
                BACKEND_KEYS.contains(&key.as_str()),
                "catalog entry `{canonical}` names unknown backend `{key}`"
            );
        }
    }
}

#[test]
fn every_name_is_one_plain_argument() {
    // Resolved names land on a root command line, one argument each.
    for (canonical, entry) in catalog() {
        let names = entry
            .iter()
            .filter(|(key, _)| key.as_str() != UPSTREAM_KEY)
            .map(|(_, name)| name)
            .chain([canonical]);
        for name in names {
            assert!(
                !name.is_empty()
                    && !name.starts_with('-')
                    && !name.chars().any(|c| c.is_whitespace() || c.is_control()),
                "catalog entry `{canonical}` has an unusable name `{name}`"
            );
        }
    }
}

#[test]
fn protobuf_compiler_resolves_per_backend() {
    const CASES: &[(&str, &str)] = &[
        ("apt", "protobuf-compiler"),
        ("dnf", "protobuf-compiler"),
        ("pacman", "protobuf"),
        ("zypper", "protobuf-devel"),
        ("apk", "protoc"),
        ("freebsd", "protobuf"),
        ("termux", "protobuf"),
        ("brew", "protobuf"),
        // Unverified managers fall through.
        ("opkg", "protobuf-compiler"),
    ];
    for &(backend, native) in CASES {
        assert_eq!(
            resolve_one(backend, "protobuf-compiler"),
            native,
            "{backend}"
        );
    }
}

#[test]
fn pkg_config_resolves_to_pkgconf_where_it_diverges() {
    const CASES: &[(&str, &str)] = &[
        ("apt", "pkg-config"),
        ("dnf", "pkgconf-pkg-config"),
        ("zypper", "pkgconf-pkg-config"),
        ("pacman", "pkgconf"),
        ("apk", "pkgconf"),
        ("freebsd", "pkgconf"),
        ("brew", "pkgconf"),
        ("termux", "pkg-config"),
    ];
    for &(backend, native) in CASES {
        assert_eq!(resolve_one(backend, "pkg-config"), native, "{backend}");
    }
}

#[test]
fn header_packages_resolve_per_backend() {
    const CASES: &[(&str, &str, &str)] = &[
        ("libsqlite3-dev", "dnf", "sqlite-devel"),
        ("libsqlite3-dev", "zypper", "sqlite3-devel"),
        ("libsqlite3-dev", "apk", "sqlite-dev"),
        ("libsqlite3-dev", "freebsd", "sqlite3"),
        ("libsqlite3-dev", "brew", "sqlite"),
        ("libssl-dev", "apk", "openssl-dev"),
        ("libssl-dev", "brew", "openssl@3"),
        // FreeBSD's base has OpenSSL; nothing to map to.
        ("libssl-dev", "freebsd", "libssl-dev"),
        ("libpq-dev", "zypper", "postgresql-devel"),
        ("libpq-dev", "brew", "libpq"),
    ];
    for &(canonical, backend, native) in CASES {
        assert_eq!(
            resolve_one(backend, canonical),
            native,
            "{canonical} on {backend}"
        );
    }
}
