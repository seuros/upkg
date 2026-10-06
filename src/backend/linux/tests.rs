use super::*;
use rstest::rstest;
use std::assert_matches;

#[rstest]
#[case("ID=ubuntu\n", "apt", "ubuntu")]
#[case("ID=debian\n", "apt", "debian")]
#[case("ID=mint\nID_LIKE=debian\n", "apt", "debian-like")]
#[case("ID=fedora\n", "dnf", "fedora")]
#[case("ID=rhel\n", "dnf", "rhel")]
#[case("ID=centos\n", "dnf", "centos")]
#[case("ID=arch\n", "pacman", "arch")]
#[case("ID=manjaro\nID_LIKE=arch\n", "pacman", "arch-like")]
#[case("ID=opensuse\n", "zypper", "opensuse")]
#[case("ID=openwrt\n", "opkg", "openwrt")]
#[case("ID=alpine\n", "apk", "alpine")]
#[case("ID=postmarketos\nID_LIKE=alpine\n", "apk", "alpine-like")]
fn detect_distro_from_os_release(
    #[case] os_release: &str,
    #[case] expected_manager: &str,
    #[case] _description: &str,
) {
    let manager = detect_from(os_release, true).unwrap();
    assert_eq!(manager.name(), expected_manager);
}

#[test]
fn detect_falls_back_to_yum_without_dnf() {
    let manager = detect_from("ID=centos\n", false).unwrap();
    assert_matches!(manager, LinuxManager::Yum);
}

#[test]
fn detect_unsupported_distro() {
    let result = detect_from("ID=unknown\n", true);
    assert!(result.is_err());
    assert_matches!(result, Err(UpkgError::Unsupported(_)));
}

#[test]
fn package_lists_need_a_packages_file() {
    let empty = ["lock", "partial", "auxfiles"].map(OsString::from);
    assert!(!lists_packages(empty));

    let fetched = [
        "lock",
        "deb.debian.org_debian_dists_bookworm_main_binary-arm64_Packages.lz4",
    ]
    .map(OsString::from);
    assert!(lists_packages(fetched));
}

#[test]
fn missing_lists_dir_has_no_package_lists() {
    assert!(!has_package_lists(Path::new("/nonexistent/upkg/apt/lists")));
}

#[test]
fn only_apt_needs_a_refresh() {
    for manager in [LinuxManager::Dnf, LinuxManager::Pacman, LinuxManager::Apk] {
        assert!(manager.refresh_spec().is_none(), "{manager:?}");
    }
}

#[rstest]
#[case(LinuxManager::Apt, vec!["git".into()], "sudo", vec!["apt", "install", "-y", "git"])]
#[case(LinuxManager::Dnf, vec!["curl".into()], "sudo", vec!["dnf", "install", "-y", "curl"])]
#[case(LinuxManager::Yum, vec!["wget".into()], "sudo", vec!["yum", "install", "-y", "wget"])]
#[case(LinuxManager::Pacman, vec!["vim".into()], "sudo", vec!["pacman", "-S", "--noconfirm", "vim"])]
#[case(LinuxManager::Zypper, vec!["gcc".into()], "sudo", vec!["zypper", "--non-interactive", "install", "gcc"])]
#[case(LinuxManager::Opkg, vec!["ca-certificates".into()], "opkg", vec!["install", "ca-certificates"])]
#[case(LinuxManager::Apk, vec!["clang-dev".into()], "sudo", vec!["apk", "add", "--update-cache", "clang-dev"])]
fn install_spec_generates_correct_commands(
    #[case] manager: LinuxManager,
    #[case] packages: Vec<String>,
    #[case] expected_command: &str,
    #[case] expected_args: Vec<&str>,
) {
    let spec = manager.install_spec(&packages);
    assert_eq!(spec.command(), expected_command);

    let args: Vec<&str> = spec.args().iter().map(|s| s.as_str()).collect();
    assert_eq!(args, expected_args);
}

#[test]
fn install_spec_handles_multiple_packages() {
    let manager = LinuxManager::Apt;
    let packages = vec!["git".into(), "curl".into(), "wget".into()];
    let spec = manager.install_spec(&packages);

    let args: Vec<&str> = spec.args().iter().map(|s| s.as_str()).collect();
    assert_eq!(args, vec!["apt", "install", "-y", "git", "curl", "wget"]);
}

#[rstest]
#[case(LinuxManager::Apt, vec!["git".into()], "sudo", vec!["apt", "remove", "-y", "git"])]
#[case(LinuxManager::Pacman, vec!["vim".into()], "sudo", vec!["pacman", "-R", "--noconfirm", "vim"])]
#[case(LinuxManager::Apk, vec!["vim".into()], "sudo", vec!["apk", "del", "vim"])]
fn uninstall_spec_generates_correct_commands(
    #[case] manager: LinuxManager,
    #[case] packages: Vec<String>,
    #[case] expected_command: &str,
    #[case] expected_args: Vec<&str>,
) {
    let spec = manager.uninstall_spec(&packages);
    assert_eq!(spec.command(), expected_command);

    let args: Vec<&str> = spec.args().iter().map(|s| s.as_str()).collect();
    assert_eq!(args, expected_args);
}

#[rstest]
#[case(LinuxManager::Apt, "sudo", vec!["apt", "install", "--reinstall", "-y", "git"])]
#[case(LinuxManager::Dnf, "sudo", vec!["dnf", "reinstall", "-y", "git"])]
#[case(LinuxManager::Yum, "sudo", vec!["yum", "reinstall", "-y", "git"])]
#[case(LinuxManager::Pacman, "sudo", vec!["pacman", "-S", "--noconfirm", "git"])]
#[case(LinuxManager::Zypper, "sudo", vec!["zypper", "--non-interactive", "install", "--force", "git"])]
#[case(LinuxManager::Opkg, "opkg", vec!["install", "--force-reinstall", "git"])]
#[case(LinuxManager::Apk, "sudo", vec!["apk", "fix", "--reinstall", "git"])]
fn reinstall_spec_generates_correct_commands(
    #[case] manager: LinuxManager,
    #[case] expected_command: &str,
    #[case] expected_args: Vec<&str>,
) {
    let spec = manager.reinstall_spec(&["git".to_string()]);
    assert_eq!(spec.command(), expected_command);

    let args: Vec<&str> = spec.args().iter().map(|s| s.as_str()).collect();
    assert_eq!(args, expected_args);
}

#[rstest]
#[case(LinuxManager::Apt, "ripgrep", "apt", vec!["search", "ripgrep"])]
#[case(LinuxManager::Dnf, "ripgrep", "dnf", vec!["search", "ripgrep"])]
#[case(LinuxManager::Yum, "ripgrep", "yum", vec!["search", "ripgrep"])]
#[case(LinuxManager::Zypper, "ripgrep", "zypper", vec!["search", "ripgrep"])]
#[case(LinuxManager::Pacman, "ripgrep", "pacman", vec!["-Ss", "ripgrep"])]
#[case(LinuxManager::Opkg, "ripgrep", "opkg", vec!["find", "ripgrep"])]
#[case(LinuxManager::Apk, "ripgrep", "apk", vec!["search", "ripgrep"])]
fn search_spec_non_exact(
    #[case] manager: LinuxManager,
    #[case] query: &str,
    #[case] expected_command: &str,
    #[case] expected_args: Vec<&str>,
) {
    let spec = manager.search_spec(query, false).expect("search ok");
    assert_eq!(spec.command(), expected_command);
    let args: Vec<&str> = spec.args().iter().map(|s| s.as_str()).collect();
    assert_eq!(args, expected_args);
}

#[test]
fn search_spec_no_sudo_wrapper() {
    let spec = LinuxManager::Apt.search_spec("git", false).unwrap();
    assert_ne!(spec.command(), "sudo");
}

#[rstest]
#[case(LinuxManager::Apt)]
#[case(LinuxManager::Dnf)]
#[case(LinuxManager::Yum)]
#[case(LinuxManager::Zypper)]
fn search_spec_rejects_exact_on_unsupported_managers(#[case] manager: LinuxManager) {
    let err = manager.search_spec("git", true).expect_err("should reject");
    assert_matches!(err, UpkgError::Unsupported(_));
}

#[test]
fn search_spec_pacman_anchors_exact_query() {
    let spec = LinuxManager::Pacman.search_spec("git", true).unwrap();
    let args: Vec<&str> = spec.args().iter().map(|s| s.as_str()).collect();
    assert_eq!(args, vec!["-Ss", "^git$"]);
}

#[test]
fn search_spec_opkg_anchors_exact_query() {
    let spec = LinuxManager::Opkg.search_spec("git", true).unwrap();
    let args: Vec<&str> = spec.args().iter().map(|s| s.as_str()).collect();
    assert_eq!(args, vec!["find", "^git$"]);
}

#[test]
fn search_spec_apk_passes_exact_flag() {
    let spec = LinuxManager::Apk.search_spec("c++.tools", true).unwrap();
    let args: Vec<&str> = spec.args().iter().map(|s| s.as_str()).collect();
    assert_eq!(args, vec!["search", "--exact", "c++.tools"]);
}

#[test]
fn search_spec_pacman_escapes_regex_metacharacters_in_exact() {
    let spec = LinuxManager::Pacman.search_spec("c++.tools", true).unwrap();
    let args: Vec<&str> = spec.args().iter().map(|s| s.as_str()).collect();
    assert_eq!(args, vec!["-Ss", r"^c\+\+\.tools$"]);
}

#[test]
fn upgrade_spec_handles_empty_packages_for_apt() {
    let manager = LinuxManager::Apt;
    let packages: Vec<String> = vec![];
    let spec = manager.upgrade_spec(&packages);

    let args: Vec<&str> = spec.args().iter().map(|s| s.as_str()).collect();
    assert_eq!(args, vec!["apt", "upgrade", "-y"]);
}

#[rstest]
#[case(vec![], vec!["apk", "upgrade", "--update-cache"])]
#[case(vec!["git".into()], vec!["apk", "add", "--update-cache", "--upgrade", "git"])]
fn upgrade_spec_apk(#[case] packages: Vec<String>, #[case] expected_args: Vec<&str>) {
    let spec = LinuxManager::Apk.upgrade_spec(&packages);
    let args: Vec<&str> = spec.args().iter().map(|s| s.as_str()).collect();
    assert_eq!(args, expected_args);
}
