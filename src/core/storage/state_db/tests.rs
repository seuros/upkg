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
