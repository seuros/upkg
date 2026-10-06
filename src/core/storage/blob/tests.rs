use super::*;
use tempfile::TempDir;

#[test]
fn completed_write_produces_final_blob() {
    let tmp = TempDir::new().unwrap();
    let cache = BlobCache::new(tmp.path()).unwrap();

    let sha = "abc123";
    let mut writer = cache.start_write(sha).unwrap();
    writer.write_all(b"hello world").unwrap();

    let final_path = writer.commit().unwrap();

    assert!(final_path.exists());
    assert!(cache.has_blob(sha));
    assert_eq!(fs::read_to_string(&final_path).unwrap(), "hello world");
}

#[test]
fn interrupted_write_leaves_no_final_blob() {
    let tmp = TempDir::new().unwrap();
    let cache = BlobCache::new(tmp.path()).unwrap();

    let sha = "def456";

    {
        let mut writer = cache.start_write(sha).unwrap();
        writer.write_all(b"partial data").unwrap();
    }

    assert!(!cache.has_blob(sha));

    let tmp_dir = tmp.path().join("tmp");
    let has_temp_files = fs::read_dir(&tmp_dir)
        .unwrap()
        .any(|e| e.unwrap().file_name().to_string_lossy().starts_with(sha));
    assert!(!has_temp_files, "temp files for {sha} should be cleaned up");
}

#[test]
fn blob_path_uses_sha256() {
    let tmp = TempDir::new().unwrap();
    let cache = BlobCache::new(tmp.path()).unwrap();

    let path = cache.blob_path("deadbeef");
    assert!(path.to_string_lossy().contains("deadbeef.tar.gz"));
}

#[test]
fn remove_blob_deletes_existing_blob() {
    let tmp = TempDir::new().unwrap();
    let cache = BlobCache::new(tmp.path()).unwrap();

    let sha = "removeme";
    let mut writer = cache.start_write(sha).unwrap();
    writer.write_all(b"corrupt data").unwrap();
    writer.commit().unwrap();

    assert!(cache.has_blob(sha));

    let removed = cache.remove_blob(sha).unwrap();
    assert!(removed);
    assert!(!cache.has_blob(sha));
}

#[test]
fn remove_blob_returns_false_for_nonexistent() {
    let tmp = TempDir::new().unwrap();
    let cache = BlobCache::new(tmp.path()).unwrap();

    let removed = cache.remove_blob("nonexistent").unwrap();
    assert!(!removed);
}
