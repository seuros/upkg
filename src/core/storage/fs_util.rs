use std::fmt::Display;
use std::fs;
use std::path::{Path, PathBuf};

use crate::types::Error;

/// Turn a failed filesystem operation into `Error::StoreCorruption` with a
/// `"<context>: <cause>"` message. The context is only built on failure.
pub(crate) trait StoreContext<T> {
    fn store_context<C: Display>(self, context: impl FnOnce() -> C) -> Result<T, Error>;
}

impl<T, E: Display> StoreContext<T> for Result<T, E> {
    fn store_context<C: Display>(self, context: impl FnOnce() -> C) -> Result<T, Error> {
        self.map_err(|e| Error::StoreCorruption {
            message: format!("{}: {e}", context()),
        })
    }
}

/// Paths of the entries in `dir`, for best-effort scans.
///
/// Opening `dir` must succeed (error: "failed to read <what> '<dir>': ...");
/// individual entries that cannot be read are skipped on purpose.
pub(crate) fn dir_paths(dir: &Path, what: &str) -> Result<impl Iterator<Item = PathBuf>, Error> {
    let entries =
        fs::read_dir(dir).store_context(|| format!("failed to read {what} '{}'", dir.display()))?;
    Ok(entries.filter_map(|entry| entry.ok().map(|entry| entry.path())))
}

/// The final path component as UTF-8, if it has one.
pub(crate) fn file_name_str(path: &Path) -> Option<&str> {
    path.file_name().and_then(|name| name.to_str())
}
