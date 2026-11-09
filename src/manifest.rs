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
//! // A checkout's own upkg.toml: a regular file, or nothing.
//! let Some(manifest) = Manifest::load_from_dir("/srv/checkout")? else {
//!     return Ok(());
//! };
//! let host = Host::detect()?;
//! let packages = manifest.resolve(host.platform().catalog_key());
//! println!("image {}: {}", packages.digest(), packages.key()); // "apt:pkg-config protobuf-compiler"
//! let options = Options { dry_run: false, privilege: Privilege::AlreadyRoot };
//! host.install(&packages, &options)?;
//! # Ok(())
//! # }
//! ```

use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::str::FromStr;

mod error;
mod parse;
mod set;

pub use error::Error;
pub use set::{Package, PackageSet};

/// The manifest's file name. `upkg` reads it from the current directory only.
pub const FILE_NAME: &str = "upkg.toml";

/// The largest manifest upkg reads, in bytes. A list of package names is a
/// few hundred; anything near this is not a manifest.
pub const MAX_LEN: u64 = 64 * 1024;

/// A parsed `upkg.toml`: canonical package names, as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    packages: Vec<String>,
}

impl Manifest {
    /// Reads and parses the manifest at `path`, wherever it leads: a path
    /// someone chose, such as `upkg install --manifest PATH`. For the
    /// `upkg.toml` of a directory whose content nobody vetted, use
    /// [`Manifest::load_from_dir`].
    ///
    /// # Errors
    ///
    /// [`Error::Read`] when the file can't be read
    /// ([`Error::is_not_found`] when there is none), [`Error::Refused`] when
    /// it is larger than [`MAX_LEN`], [`Error::Invalid`] when it isn't a
    /// valid manifest.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref();
        let file = File::open(path).map_err(|source| Error::Read {
            path: path.to_path_buf(),
            source,
        })?;
        Self::read(file, path)
    }

    /// Reads `<dir>/upkg.toml`, if there is one, as a root process may read
    /// a file from a checkout it doesn't trust: the file must be a regular
    /// file (a symlink is refused, never followed), no larger than
    /// [`MAX_LEN`], and errors quote none of it beyond a key name.
    ///
    /// `Ok(None)` when there is no `upkg.toml`; a manifest listing no
    /// packages is `Some` of an empty one.
    ///
    /// # Errors
    ///
    /// [`Error::Refused`] for a symlink, a directory, a device, a file
    /// larger than [`MAX_LEN`], or one swapped for another while it was
    /// opened; [`Error::Read`] when it can't be read; [`Error::Invalid`]
    /// when it isn't a valid manifest.
    pub fn load_from_dir(dir: impl AsRef<Path>) -> Result<Option<Self>, Error> {
        let path = dir.as_ref().join(FILE_NAME);
        let read = |source| Error::Read {
            path: path.clone(),
            source,
        };
        let refused = |reason| Error::Refused {
            path: path.clone(),
            reason,
        };
        let checked = match std::fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(read(error)),
            Ok(meta) => meta,
        };
        if !checked.file_type().is_file() {
            return Err(refused(NOT_A_FILE));
        }
        if checked.len() > MAX_LEN {
            return Err(refused(TOO_LARGE));
        }
        let file = File::open(&path).map_err(read)?;
        // The name could have become a symlink since it was looked at:
        // what was opened must be the file that was checked.
        if !same_file(&checked, &file.metadata().map_err(read)?) {
            return Err(refused(SWAPPED));
        }
        Self::read(file, &path).map(Some)
    }

    /// At most [`MAX_LEN`] bytes of `file`, parsed.
    fn read(file: File, path: &Path) -> Result<Self, Error> {
        let mut bytes = Vec::new();
        file.take(MAX_LEN + 1)
            .read_to_end(&mut bytes)
            .map_err(|source| Error::Read {
                path: path.to_path_buf(),
                source,
            })?;
        if bytes.len() as u64 > MAX_LEN {
            return Err(Error::Refused {
                path: path.to_path_buf(),
                reason: TOO_LARGE,
            });
        }
        let text = String::from_utf8(bytes).map_err(|_| Error::Invalid {
            path: Some(path.to_path_buf()),
            message: "not UTF-8 text".to_string(),
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

const NOT_A_FILE: &str =
    "is not a regular file (a symlink, a directory or a device); upkg reads only a plain upkg.toml";
const TOO_LARGE: &str = "is larger than 64 KiB, which no list of package names is";
const SWAPPED: &str = "was replaced while it was being opened";

/// Whether `opened` is the file `checked` described.
#[cfg(unix)]
fn same_file(checked: &std::fs::Metadata, opened: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    checked.dev() == opened.dev() && checked.ino() == opened.ino()
}

/// Whether `opened` is the file `checked` described; without inode numbers,
/// at least still a regular file.
#[cfg(not(unix))]
fn same_file(_checked: &std::fs::Metadata, opened: &std::fs::Metadata) -> bool {
    opened.is_file()
}

impl FromStr for Manifest {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self, Error> {
        let packages = parse::packages(text).map_err(|message| Error::Invalid {
            path: None,
            message,
        })?;
        Ok(Self { packages })
    }
}

#[cfg(test)]
mod tests;
