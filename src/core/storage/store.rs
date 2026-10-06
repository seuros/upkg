use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};

use crate::core::extraction::extract::extract_archive;
use crate::types::Error;

pub struct Store {
    store_dir: PathBuf,
    locks_dir: PathBuf,
}

impl Store {
    pub fn new(root: &Path) -> io::Result<Self> {
        let store_dir = root.join("store");
        let locks_dir = root.join("locks");

        fs::create_dir_all(&store_dir)?;
        fs::create_dir_all(&locks_dir)?;

        Ok(Self {
            store_dir,
            locks_dir,
        })
    }

    pub fn entry_path(&self, store_key: &str) -> PathBuf {
        self.store_dir.join(store_key)
    }

    pub fn root_dir(&self) -> &Path {
        &self.store_dir
    }

    pub fn has_entry(&self, store_key: &str) -> bool {
        self.entry_path(store_key).exists()
    }

    pub fn ensure_entry(&self, store_key: &str, blob_path: &Path) -> Result<PathBuf, Error> {
        let entry_path = self.entry_path(store_key);

        if entry_path.exists() {
            return Ok(entry_path);
        }

        let _lock = self.lock_entry(store_key)?;

        if entry_path.exists() {
            return Ok(entry_path);
        }

        let tmp_dir = self
            .store_dir
            .join(format!(".{store_key}.tmp.{}", std::process::id()));

        if tmp_dir.exists() {
            let _ = fs::remove_dir_all(&tmp_dir);
        }

        fs::create_dir_all(&tmp_dir).map_err(|e| Error::StoreCorruption {
            message: format!("failed to create temp directory: {e}"),
        })?;

        if let Err(e) = extract_archive(blob_path, &tmp_dir) {
            let _ = fs::remove_dir_all(&tmp_dir);
            return Err(e);
        }

        if let Err(e) = fs::rename(&tmp_dir, &entry_path) {
            let _ = fs::remove_dir_all(&tmp_dir);
            return Err(Error::StoreCorruption {
                message: format!("failed to rename store entry: {e}"),
            });
        }

        Ok(entry_path)
    }

    fn lock_path(&self, store_key: &str) -> PathBuf {
        self.locks_dir.join(format!("{store_key}.lock"))
    }

    fn lock_entry(&self, store_key: &str) -> Result<File, Error> {
        let lock_file =
            File::create(self.lock_path(store_key)).map_err(|e| Error::StoreCorruption {
                message: format!("failed to create lock file: {e}"),
            })?;

        lock_file.lock().map_err(|e| Error::StoreCorruption {
            message: format!("failed to acquire lock: {e}"),
        })?;

        Ok(lock_file)
    }

    pub fn remove_entry(&self, store_key: &str) -> Result<(), Error> {
        let entry_path = self.entry_path(store_key);

        if !entry_path.exists() {
            return Ok(());
        }

        let _lock = self.lock_entry(store_key)?;

        if entry_path.exists() {
            fs::remove_dir_all(&entry_path).map_err(|e| Error::StoreCorruption {
                message: format!("failed to remove store entry: {e}"),
            })?;
        }

        let _ = fs::remove_file(self.lock_path(store_key));

        Ok(())
    }
}

#[cfg(all(test, target_os = "macos"))]
#[path = "store/tests.rs"]
mod tests;
