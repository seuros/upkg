//! Cross-manager package name aliasing.
//!
//! The catalog is a small, compiled-in, human-reviewed table of packages whose
//! name DIVERGES across managers (e.g. `libpq-dev` / `libpq-devel` /
//! `postgresql-libs`). It is deliberately NOT a registry: anything that already
//! has the same name everywhere is resolved by verbatim fallthrough, never by
//! an entry. See `catalog/aliases.toml` for the contribution policy.
//!
//! Resolution is exact-match only. There is no fuzzy matching, which is what
//! keeps this from becoming a typo-squat redirect surface.

use std::collections::BTreeMap;
use std::sync::OnceLock;

/// Reserved key inside an entry: identifies the upstream project so reviewers
/// can verify all per-manager names are the same software. Never a backend key.
const UPSTREAM_KEY: &str = "upstream";

const RAW: &str = include_str!("catalog/aliases.toml");

/// One catalog entry: a flat map of `backend_key -> native name`, plus the
/// reserved `upstream` key. Modeling the whole table as a string map keeps the
/// parser bulletproof (no serde flatten edge cases) and lets new backend keys
/// be added in TOML alone.
type Entry = BTreeMap<String, String>;

fn catalog() -> &'static BTreeMap<String, Entry> {
    static CATALOG: OnceLock<BTreeMap<String, Entry>> = OnceLock::new();
    CATALOG.get_or_init(|| toml::from_str(RAW).expect("embedded alias catalog must be valid TOML"))
}

/// Resolve a single canonical name to the native name for `backend_key`.
///
/// Returns the catalog value when (and only when) the exact canonical name has
/// an entry that names this backend; otherwise the input is returned unchanged.
pub fn resolve_one(backend_key: &str, name: &str) -> String {
    debug_assert_ne!(backend_key, UPSTREAM_KEY, "`upstream` is not a backend key");
    catalog()
        .get(name)
        .and_then(|entry| entry.get(backend_key))
        .cloned()
        .unwrap_or_else(|| name.to_string())
}

/// Resolve a list of canonical names for `backend_key`, preserving order.
pub fn resolve(backend_key: &str, packages: &[String]) -> Vec<String> {
    packages
        .iter()
        .map(|name| resolve_one(backend_key, name))
        .collect()
}

#[cfg(test)]
mod tests;
