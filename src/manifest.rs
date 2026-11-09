//! `upkg.toml`: the system packages a project needs, in the names people
//! already know.
//!
//! ```toml
//! packages = ["protobuf-compiler", "pkg-config"]
//! ```
//!
//! Names go through the alias catalog like any other: `protobuf-compiler` is
//! `protobuf` to FreeBSD's pkg and `protoc` to apk, and a name the catalog
//! doesn't know reaches the native manager verbatim. Any key but `packages` is
//! an error, so a typo never silently installs nothing.
//!
//! ```no_run
//! use upkg::host::{Host, Options, Privilege};
//! use upkg::manifest::Manifest;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let manifest = Manifest::load("upkg.toml")?;
//! let host = Host::detect()?;
//! let packages = manifest.resolve(host.platform().catalog_key());
//! println!("image key: {}", packages.key()); // "apt:pkg-config protobuf-compiler"
//! let options = Options { dry_run: false, privilege: Privilege::AlreadyRoot };
//! host.install(&packages, &options)?;
//! # Ok(())
//! # }
//! ```

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::catalog;

/// The manifest's file name. `upkg` reads it from the current directory only.
pub const FILE_NAME: &str = "upkg.toml";

/// A parsed `upkg.toml`: canonical package names, as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    packages: Vec<String>,
}

/// The file as TOML sees it. New keys (per-OS lists, say) are added here;
/// until then they are errors.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    packages: Vec<String>,
}

impl Manifest {
    /// Reads and parses the manifest at `path`.
    ///
    /// # Errors
    ///
    /// [`Error::Read`] when the file can't be read (its `source` says
    /// [`io::ErrorKind::NotFound`] when there is none), [`Error::Invalid`]
    /// when it isn't a valid manifest.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|source| Error::Read {
            path: path.to_path_buf(),
            source,
        })?;
        text.parse().map_err(|error: Error| error.at(path))
    }

    /// The canonical names, in file order, duplicates kept.
    #[must_use]
    pub fn packages(&self) -> &[String] {
        &self.packages
    }

    /// The packages as `backend` (a catalog key, see
    /// [`Platform::catalog_key`](crate::host::Platform::catalog_key)) names
    /// them.
    #[must_use]
    pub fn resolve(&self, backend: &str) -> PackageSet {
        PackageSet::resolve(backend, &self.packages)
    }
}

impl FromStr for Manifest {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self, Error> {
        let raw: Raw = toml::from_str(text).map_err(|error| Error::Invalid {
            path: None,
            message: error.to_string().trim_end().to_string(),
        })?;
        for name in &raw.packages {
            check_name(name).map_err(|message| Error::Invalid {
                path: None,
                message,
            })?;
        }
        Ok(Self {
            packages: raw.packages,
        })
    }
}

/// A name must reach the native manager as one plain argument: never empty,
/// never an option, never split by a shell.
fn check_name(name: &str) -> Result<(), String> {
    let problem = if name.is_empty() {
        "is empty"
    } else if name.starts_with('-') {
        "starts with `-`; a package manager would read it as an option"
    } else if name.chars().any(|c| c.is_whitespace() || c.is_control()) {
        "contains whitespace or a control character"
    } else {
        return Ok(());
    };
    Err(format!("package name {name:?} {problem}"))
}

/// Packages resolved for one backend: native names, sorted and deduplicated.
///
/// Serializes as `{"backend": "freebsd", "packages": ["pkgconf", "protobuf"]}`;
/// deserializing sorts, deduplicates and checks the names again, so a set that
/// crossed a process boundary holds what [`PackageSet::key`] promises.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "RawSet")]
pub struct PackageSet {
    backend: String,
    packages: Vec<String>,
}

#[derive(Deserialize)]
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
            check_name(name)?;
        }
        Ok(Self::native(raw.backend, raw.packages))
    }
}

impl PackageSet {
    /// Resolves canonical `packages` for `backend` (a catalog key) through
    /// the alias catalog.
    #[must_use]
    pub fn resolve(backend: &str, packages: &[String]) -> Self {
        Self::native(backend.to_string(), catalog::resolve(backend, packages))
    }

    fn native(backend: String, mut packages: Vec<String>) -> Self {
        packages.sort_unstable();
        packages.dedup();
        Self { backend, packages }
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
}

/// Why a manifest could not be loaded.
#[derive(Debug)]
pub enum Error {
    /// The file could not be read.
    Read { path: PathBuf, source: io::Error },
    /// Not TOML, an unknown key, a wrong type, or an unusable package name.
    Invalid {
        /// The file, when the manifest came from one.
        path: Option<PathBuf>,
        message: String,
    },
}

impl Error {
    fn at(self, file: &Path) -> Self {
        match self {
            Self::Invalid {
                path: None,
                message,
            } => Self::Invalid {
                path: Some(file.to_path_buf()),
                message,
            },
            other => other,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => write!(f, "{}: {source}", path.display()),
            Self::Invalid {
                path: Some(path),
                message,
            } => write!(f, "{}: {message}", path.display()),
            Self::Invalid {
                path: None,
                message,
            } => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            Self::Invalid { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests;
