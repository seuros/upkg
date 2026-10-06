use super::*;
use std::assert_matches;
use std::os::unix::fs::PermissionsExt;
use tempfile::TempDir;

fn setup_store_entry(tmp: &TempDir) -> PathBuf {
    let store_entry = tmp.path().join("store/abc123");

    fs::create_dir_all(store_entry.join("bin")).unwrap();
    fs::create_dir_all(store_entry.join("lib")).unwrap();

    fs::write(store_entry.join("bin/foo"), b"#!/bin/sh\necho foo").unwrap();
    let mut perms = fs::metadata(store_entry.join("bin/foo"))
        .unwrap()
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(store_entry.join("bin/foo"), perms).unwrap();

    fs::write(store_entry.join("lib/libfoo.dylib"), b"fake dylib").unwrap();

    std::os::unix::fs::symlink("libfoo.dylib", store_entry.join("lib/libfoo.1.dylib")).unwrap();

    store_entry
}

#[test]
fn tree_reproduced_exactly() {
    let tmp = TempDir::new().unwrap();
    let store_entry = setup_store_entry(&tmp);

    let cellar = Cellar::new(tmp.path()).unwrap();
    let keg_path = cellar.materialize("foo", "1.2.3", &store_entry).unwrap();

    assert!(keg_path.exists());
    assert!(keg_path.join("bin").exists());
    assert!(keg_path.join("lib").exists());

    assert_eq!(
        fs::read_to_string(keg_path.join("bin/foo")).unwrap(),
        "#!/bin/sh\necho foo"
    );
    assert_eq!(
        fs::read(keg_path.join("lib/libfoo.dylib")).unwrap(),
        b"fake dylib"
    );

    let perms = fs::metadata(keg_path.join("bin/foo"))
        .unwrap()
        .permissions();
    assert!(perms.mode() & 0o111 != 0, "executable bit not preserved");

    let link_path = keg_path.join("lib/libfoo.1.dylib");
    assert!(
        link_path
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read_link(&link_path).unwrap(),
        PathBuf::from("libfoo.dylib")
    );
}

#[test]
fn empty_store_entry_is_rejected() {
    let tmp = TempDir::new().unwrap();
    let store_entry = tmp.path().join("store/abc");
    fs::create_dir_all(&store_entry).unwrap();

    let cellar = Cellar::new(tmp.path()).unwrap();
    let err = cellar
        .materialize("foo", "1.2.3", &store_entry)
        .unwrap_err();

    assert_matches!(err, Error::StoreCorruption { .. });
    assert!(!cellar.keg_path("foo", "1.2.3").exists());
}

#[test]
fn second_materialize_is_noop() {
    let tmp = TempDir::new().unwrap();
    let store_entry = setup_store_entry(&tmp);

    let cellar = Cellar::new(tmp.path()).unwrap();

    let keg_path1 = cellar.materialize("foo", "1.2.3", &store_entry).unwrap();

    fs::write(keg_path1.join("marker.txt"), b"original").unwrap();

    let keg_path2 = cellar.materialize("foo", "1.2.3", &store_entry).unwrap();
    assert_eq!(keg_path1, keg_path2);

    assert!(keg_path2.join("marker.txt").exists());
}

#[test]
fn remove_keg_cleans_up() {
    let tmp = TempDir::new().unwrap();
    let store_entry = setup_store_entry(&tmp);

    let cellar = Cellar::new(tmp.path()).unwrap();
    cellar.materialize("foo", "1.2.3", &store_entry).unwrap();

    assert!(cellar.has_keg("foo", "1.2.3"));

    cellar.remove_keg("foo", "1.2.3").unwrap();

    assert!(!cellar.has_keg("foo", "1.2.3"));
}

#[test]
fn keg_path_format() {
    let tmp = TempDir::new().unwrap();
    let cellar = Cellar::new(tmp.path()).unwrap();

    let path = cellar.keg_path("libheif", "2.0.1");
    assert!(path.ends_with("Cellar/libheif/2.0.1"));
}

#[test]
fn hardlink_fallback_to_copy_works() {
    let tmp1 = TempDir::new().unwrap();
    let tmp2 = TempDir::new().unwrap();

    let src = tmp1.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("test.txt"), b"test content").unwrap();

    let dst = tmp2.path().join("dst");

    copy_dir_copy_only(&src, &dst).unwrap();

    assert_eq!(
        fs::read_to_string(dst.join("test.txt")).unwrap(),
        "test content"
    );
}

#[test]
#[cfg(target_os = "macos")]
fn clonefile_fallback_works() {
    let tmp = TempDir::new().unwrap();
    let store_entry = setup_store_entry(&tmp);

    let cellar = Cellar::new(tmp.path()).unwrap();
    let keg_path = cellar.materialize("clone", "1.0.0", &store_entry).unwrap();

    assert_eq!(
        fs::read_to_string(keg_path.join("bin/foo")).unwrap(),
        "#!/bin/sh\necho foo"
    );
}
