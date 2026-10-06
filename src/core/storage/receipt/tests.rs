use super::*;
use tempfile::TempDir;

#[test]
fn empty_version_dir_is_not_installed() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("gmp/6.3.0")).unwrap();

    assert!(scan_installed(tmp.path()).unwrap().is_empty());
    assert!(find_installed(tmp.path(), "gmp").is_none());
}

#[test]
fn populated_keg_without_receipt_is_adopted() {
    let tmp = TempDir::new().unwrap();
    let lib = tmp.path().join("libyaml/0.2.5/lib");
    fs::create_dir_all(&lib).unwrap();
    fs::write(lib.join("libyaml-0.2.dylib"), b"").unwrap();

    let keg = find_installed(tmp.path(), "libyaml").expect("adopted keg");
    assert_eq!(keg.version, "0.2.5");
    assert!(keg.store_key.is_empty());
}

#[test]
fn keg_backups_are_not_installed() {
    let tmp = TempDir::new().unwrap();
    let lib = tmp.path().join("jq/1.8.1.upkg-backup-42/lib");
    fs::create_dir_all(&lib).unwrap();
    fs::write(lib.join("libjq.1.dylib"), b"").unwrap();

    assert!(scan_installed(tmp.path()).unwrap().is_empty());
}

#[test]
fn receipt_wins_over_directory_name() {
    let tmp = TempDir::new().unwrap();
    let keg_path = tmp.path().join("openssl@3/3.6.2");
    fs::create_dir_all(&keg_path).unwrap();
    write_receipt(
        &keg_path,
        &InstallReceipt {
            install_name: "openssl@3".into(),
            formula_name: "openssl@3".into(),
            version: "3.6.2".into(),
            store_key: "abc".into(),
            installed_at: 0,
        },
    )
    .unwrap();

    let keg = find_installed(tmp.path(), "openssl@3").unwrap();
    assert_eq!(keg.store_key, "abc");
}
