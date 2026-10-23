use super::*;

#[test]
fn records_lists_and_removes_installed_packages() {
    let db = StateDb::in_memory().unwrap();
    db.record_installed(&InstalledPackage {
        name: "cask:ghostty".to_string(),
        formula_name: "cask:ghostty".to_string(),
        version: "1.3.1".to_string(),
        store_key: "abc".to_string(),
        kind: InstalledPackageKind::App,
        installed_at: 1,
    })
    .unwrap();

    let installed = db.list_installed().unwrap();
    assert_eq!(installed.len(), 1);
    assert_eq!(installed[0].name, "cask:ghostty");
    assert_eq!(installed[0].kind, InstalledPackageKind::App);

    db.remove_installed("cask:ghostty").unwrap();
    assert!(db.list_installed().unwrap().is_empty());
}

#[test]
fn open_read_only_never_creates_the_db() {
    let dir = tempfile::TempDir::new().unwrap();
    let db = path(dir.path());

    assert!(StateDb::open_read_only(&db).unwrap().is_none());
    assert!(!dir.path().join("db").exists());

    StateDb::open(&db).unwrap();
    let read_only = StateDb::open_read_only(&db).unwrap().unwrap();
    assert_eq!(read_only.list_installed().unwrap(), Vec::new());
}
