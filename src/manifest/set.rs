use std::fmt;
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize, Serializer};

use super::parse::check_name;
use crate::catalog;

/// One package of a [`PackageSet`]: the name it was asked for by, and the
/// name the backend knows it by.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Package {
    // Field order is the sort order: native first, like the set's names.
    native: String,
    requested: String,
}

impl Package {
    /// As the manifest (or the command line) spelled it: `protobuf-compiler`.
    #[must_use]
    pub fn requested(&self) -> &str {
        &self.requested
    }

    /// As the backend knows it: `protobuf` on FreeBSD.
    #[must_use]
    pub fn native(&self) -> &str {
        &self.native
    }

    /// Whether the catalog renamed it for this backend.
    #[must_use]
    pub fn is_alias(&self) -> bool {
        self.requested != self.native
    }
}

/// `protobuf-compiler (protobuf)` when the catalog renamed it, `git` when
/// it didn't.
impl fmt::Display for Package {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_alias() {
            write!(f, "{} ({})", self.requested, self.native)
        } else {
            f.write_str(&self.native)
        }
    }
}

/// Packages resolved for one backend: native names, sorted and deduplicated,
/// each remembering the name it was asked for by ([`PackageSet::entries`]).
///
/// The set is its backend and its native names: two sets asking for the same
/// packages under different spellings are equal, hash alike and share a
/// [`PackageSet::key`]. The requested names ride along for messages.
///
/// Serializes as `{"backend": "freebsd", "packages": ["pkgconf", "protobuf"]}`,
/// native names only, the shape it has always had. Deserializing sorts,
/// deduplicates and checks the names again, so a set that crossed a process
/// boundary holds what [`PackageSet::key`] promises; its entries name each
/// package by its native name, the requested ones having stayed behind.
#[derive(Debug, Clone, Deserialize)]
#[serde(try_from = "RawSet")]
pub struct PackageSet {
    backend: String,
    /// Native names, sorted and unique.
    packages: Vec<String>,
    /// Sorted by native name, then requested; unique.
    entries: Vec<Package>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSet {
    backend: String,
    packages: Vec<String>,
}

impl TryFrom<RawSet> for PackageSet {
    type Error = String;

    fn try_from(raw: RawSet) -> Result<Self, String> {
        if raw.backend.is_empty()
            || raw
                .backend
                .chars()
                .any(|c| c == ':' || c.is_whitespace() || c.is_control())
        {
            return Err(format!("backend {:?} is not a catalog key", raw.backend));
        }
        for name in &raw.packages {
            check_name(name).map_err(|problem| format!("package name {name:?} {problem}"))?;
        }
        let entries = raw
            .packages
            .into_iter()
            .map(|native| Package {
                requested: native.clone(),
                native,
            })
            .collect();
        Ok(Self::from_entries(raw.backend, entries))
    }
}

impl Serialize for PackageSet {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        RawSet {
            backend: self.backend.clone(),
            packages: self.packages.clone(),
        }
        .serialize(serializer)
    }
}

impl PartialEq for PackageSet {
    fn eq(&self, other: &Self) -> bool {
        self.backend == other.backend && self.packages == other.packages
    }
}

impl Eq for PackageSet {}

impl Hash for PackageSet {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.backend.hash(state);
        self.packages.hash(state);
    }
}

impl PackageSet {
    /// Resolves canonical `packages` for `backend` (a catalog key) through
    /// the alias catalog.
    #[must_use]
    pub fn resolve(backend: &str, packages: &[String]) -> Self {
        let entries = packages
            .iter()
            .map(|requested| Package {
                native: catalog::resolve_one(backend, requested),
                requested: requested.clone(),
            })
            .collect();
        Self::from_entries(backend.to_string(), entries)
    }

    fn from_entries(backend: String, mut entries: Vec<Package>) -> Self {
        entries.sort_unstable();
        entries.dedup();
        let mut packages: Vec<String> = entries.iter().map(|entry| entry.native.clone()).collect();
        packages.dedup();
        Self {
            backend,
            packages,
            entries,
        }
    }

    /// The catalog key the names were resolved for.
    #[must_use]
    pub fn backend(&self) -> &str {
        &self.backend
    }

    /// Native names, sorted and deduplicated.
    #[must_use]
    pub fn packages(&self) -> &[String] {
        &self.packages
    }

    /// Every distinct name asked for, with its native name; sorted by native
    /// name, then requested. Two names the catalog folds into one native
    /// package (`pkg-config` and `pkgconf` on FreeBSD) are two entries.
    #[must_use]
    pub fn entries(&self) -> &[Package] {
        &self.entries
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.packages.is_empty()
    }

    /// One string naming exactly this set: the backend, a colon, then the
    /// native names joined by single spaces (`freebsd:pkgconf protobuf`;
    /// `apt:` when empty). Equal sets give equal keys whatever order or
    /// spelling the manifest used, so it can key a cache of prepared images.
    ///
    /// Stable across upkg releases; changing the format is a breaking change.
    /// The names inside it follow the alias catalog, so a release that maps a
    /// name differently yields a different key, as it should: that is a
    /// different set of packages. It does not identify the distribution or
    /// its release (Debian 12 and Ubuntu 24.04 are both `apt`); fold the base
    /// image's own identity in alongside it.
    #[must_use]
    pub fn key(&self) -> String {
        format!("{}:{}", self.backend, self.packages.join(" "))
    }

    /// [`PackageSet::key`] in eight lowercase hex digits, short enough for an
    /// image or directory name: the 32-bit FNV-1a hash (offset basis
    /// `0x811c9dc5`, prime `0x01000193`) of the key's UTF-8 bytes.
    /// `freebsd:pkgconf protobuf` is `3ce9857d`.
    ///
    /// Stable like the key: same key, same digest, in every release. It is a
    /// name, not an identity: 32 bits collide, so whatever a digest names
    /// must also record the set (or its key) and compare it before trusting
    /// a match.
    #[must_use]
    pub fn digest(&self) -> String {
        format!("{:08x}", fnv1a(self.key().as_bytes()))
    }
}

/// 32-bit FNV-1a.
fn fnv1a(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811c_9dc5, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193)
    })
}

#[cfg(test)]
mod tests;
