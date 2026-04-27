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

        let lock_path = self.locks_dir.join(format!("{store_key}.lock"));
        let lock_file = File::create(&lock_path).map_err(|e| Error::StoreCorruption {
            message: format!("failed to create lock file: {e}"),
        })?;

        lock_file.lock().map_err(|e| Error::StoreCorruption {
            message: format!("failed to acquire lock: {e}"),
        })?;

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

    pub fn remove_entry(&self, store_key: &str) -> Result<(), Error> {
        let entry_path = self.entry_path(store_key);

        if !entry_path.exists() {
            return Ok(());
        }

        let lock_path = self.locks_dir.join(format!("{store_key}.lock"));
        let lock_file = File::create(&lock_path).map_err(|e| Error::StoreCorruption {
            message: format!("failed to create lock file: {e}"),
        })?;

        lock_file.lock().map_err(|e| Error::StoreCorruption {
            message: format!("failed to acquire lock: {e}"),
        })?;

        if entry_path.exists() {
            fs::remove_dir_all(&entry_path).map_err(|e| Error::StoreCorruption {
                message: format!("failed to remove store entry: {e}"),
            })?;
        }

        let _ = fs::remove_file(&lock_path);

        Ok(())
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::Write;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use tar::Builder;
    use tempfile::TempDir;

    fn create_test_tarball(content: &[u8]) -> Vec<u8> {
        let mut builder = Builder::new(Vec::new());

        let mut header = tar::Header::new_gnu();
        header.set_path("test.txt").unwrap();
        header.set_size(content.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append(&header, content).unwrap();

        let tar_data = builder.into_inner().unwrap();

        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&tar_data).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn second_call_is_noop() {
        let tmp = TempDir::new().unwrap();
        let store = Store::new(tmp.path()).unwrap();

        let tarball = create_test_tarball(b"hello world");
        let blob_path = tmp.path().join("test.tar.gz");
        fs::write(&blob_path, &tarball).unwrap();

        let store_key = "abc123";

        let path1 = store.ensure_entry(store_key, &blob_path).unwrap();
        assert!(path1.exists());
        assert!(path1.join("test.txt").exists());

        fs::write(path1.join("marker.txt"), "original").unwrap();

        let path2 = store.ensure_entry(store_key, &blob_path).unwrap();
        assert_eq!(path1, path2);

        assert!(path2.join("marker.txt").exists());
    }

    #[test]
    fn concurrent_calls_unpack_once() {
        let tmp = TempDir::new().unwrap();
        let store = Arc::new(Store::new(tmp.path()).unwrap());

        let tarball = create_test_tarball(b"concurrent test");
        let blob_path = tmp.path().join("test.tar.gz");
        fs::write(&blob_path, &tarball).unwrap();

        let store_key = "concurrent123";
        let unpack_count = Arc::new(AtomicUsize::new(0));

        let handles: Vec<_> = (0..10)
            .map(|_| {
                let store = store.clone();
                let blob = blob_path.clone();
                let count = unpack_count.clone();
                let key = store_key.to_string();

                thread::spawn(move || {
                    let entry_path = store.entry_path(&key);
                    let existed_before = entry_path.exists();

                    let result = store.ensure_entry(&key, &blob);

                    if !existed_before && result.is_ok() && entry_path.exists() {
                        count.fetch_add(1, Ordering::SeqCst);
                    }

                    result
                })
            })
            .collect();

        for handle in handles {
            let result = handle.join().unwrap();
            assert!(result.is_ok());
        }

        assert!(store.has_entry(store_key));

        let entry_path = store.entry_path(store_key);
        let content = fs::read_to_string(entry_path.join("test.txt")).unwrap();
        assert_eq!(content, "concurrent test");
    }

    #[test]
    fn has_entry_returns_correct_state() {
        let tmp = TempDir::new().unwrap();
        let store = Store::new(tmp.path()).unwrap();

        let store_key = "checkme";

        assert!(!store.has_entry(store_key));

        let tarball = create_test_tarball(b"exists");
        let blob_path = tmp.path().join("test.tar.gz");
        fs::write(&blob_path, &tarball).unwrap();

        store.ensure_entry(store_key, &blob_path).unwrap();

        assert!(store.has_entry(store_key));
    }
}
