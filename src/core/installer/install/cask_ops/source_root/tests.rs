use super::*;
use tempfile::TempDir;

#[test]
fn detects_dmg_by_udif_trailer_when_url_and_cache_path_hide_extension() {
    let tmp = TempDir::new().unwrap();
    let blob_path = tmp.path().join("sha256.tar.gz");
    let mut bytes = vec![0; 1024];
    let trailer_start = bytes.len() - 512;
    bytes[trailer_start..trailer_start + 4].copy_from_slice(b"koly");
    fs::write(&blob_path, bytes).unwrap();

    assert!(is_dmg(
        "https://portswigger-cdn.net/burp/releases/download?product=community&type=MacOsArm64",
        &blob_path
    ));
}

#[test]
fn does_not_detect_regular_tarball_cache_path_as_dmg() {
    let tmp = TempDir::new().unwrap();
    let blob_path = tmp.path().join("sha256.tar.gz");
    fs::write(&blob_path, vec![0; 1024]).unwrap();

    assert!(!is_dmg("https://example.com/archive.tar.gz", &blob_path));
}
