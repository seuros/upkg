use super::*;

#[test]
fn failure_context_uses_formula_named_by_error() {
    let requested = vec!["zzz".to_string(), "agent-safehouse".to_string()];
    let formula_names = vec![
        ("zzz".to_string(), "zzz".to_string()),
        ("agent-safehouse".to_string(), "agent-safehouse".to_string()),
    ];
    let error = Error::MissingFormula {
        name: "agent-safehouse".to_string(),
    };

    assert_eq!(
        crate::native_cli::commands::install::failure_context_for_error(
            &error,
            &formula_names,
            &requested
        ),
        "agent-safehouse"
    );
}

#[test]
fn dependency_cellar_path_uses_formula_token_for_tap_name() {
    let tmp = TempDir::new().unwrap();
    let cellar = Cellar::new(tmp.path()).unwrap();
    let path = dependency_cellar_path(&cellar, "hashicorp/tap/terraform", "1.10.0");

    assert!(path.ends_with("Cellar/terraform/1.10.0"));
}

#[test]
fn dependency_cellar_path_keeps_core_formula_name() {
    let tmp = TempDir::new().unwrap();
    let cellar = Cellar::new(tmp.path()).unwrap();
    let path = dependency_cellar_path(&cellar, "openssl@3", "3.3.2");

    assert!(path.ends_with("Cellar/openssl@3/3.3.2"));
}

#[test]
fn dependency_cellar_path_uses_name_from_install_record() {
    let tmp = TempDir::new().unwrap();
    let cellar = Cellar::new(tmp.path()).unwrap();
    let path = dependency_cellar_path(&cellar, "hashicorp/tap/terraform", "1.10.0");

    assert!(path.ends_with("Cellar/terraform/1.10.0"));
}

#[test]
fn source_keg_backup_can_restore_previous_installation() {
    let tmp = TempDir::new().unwrap();
    let keg_path = tmp.path().join("Cellar").join("example").join("1.0.0");
    fs::create_dir_all(&keg_path).unwrap();
    fs::write(keg_path.join("old.txt"), "old").unwrap();

    let backup = Installer::backup_existing_source_keg(&keg_path, "example", "1.0.0").unwrap();
    let backup = backup.expect("backup path should exist");

    assert!(!keg_path.exists());
    assert!(backup.exists());

    fs::create_dir_all(&keg_path).unwrap();
    fs::write(keg_path.join("new.txt"), "new").unwrap();

    Installer::restore_source_keg_from_backup(&keg_path, &backup, "example", "1.0.0").unwrap();

    assert!(keg_path.join("old.txt").exists());
    assert!(!keg_path.join("new.txt").exists());
    assert!(!backup.exists());
}

#[test]
fn backup_existing_source_keg_returns_none_when_keg_is_missing() {
    let tmp = TempDir::new().unwrap();
    let missing_keg = tmp.path().join("Cellar").join("example").join("1.0.0");

    let backup = Installer::backup_existing_source_keg(&missing_keg, "example", "1.0.0").unwrap();

    assert!(backup.is_none());
}
