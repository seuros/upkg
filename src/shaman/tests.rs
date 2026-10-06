use super::*;

#[test]
fn directory_check_fails_for_missing_directory() {
    let check = directory_check("test", Path::new("/definitely/missing/upkg/path"));
    assert_eq!(check.status, HealthStatus::Fail);
    assert!(check.detail.contains("missing"));
}

#[test]
fn find_command_finds_rustc() {
    assert!(find_command("rustc").is_some());
}
