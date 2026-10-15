use super::*;
use crate::core::extraction::test_support::tar_gz;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use tempfile::TempDir;

/// Writes a blob holding a single `test.txt` with `content` to `<tmp>/test.tar.gz`.
fn write_blob(tmp: &TempDir, content: &[u8]) -> PathBuf {
    let blob_path = tmp.path().join("test.tar.gz");
    fs::write(&blob_path, tar_gz(&[("test.txt", content, 0o644)])).unwrap();
    blob_path
}

#[test]
fn second_call_is_noop() {
    let tmp = TempDir::new().unwrap();
    let store = Store::new(tmp.path()).unwrap();

    let blob_path = write_blob(&tmp, b"hello world");

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

    let blob_path = write_blob(&tmp, b"concurrent test");

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

    let blob_path = write_blob(&tmp, b"exists");

    store.ensure_entry(store_key, &blob_path).unwrap();

    assert!(store.has_entry(store_key));
}
