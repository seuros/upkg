use std::ffi::OsString;
use std::fs;
use std::path::Path;

use crate::backend::{CommandSpec, command_exists};
use crate::error::UpkgError;

/// Where apt keeps downloaded package lists; container images ship it empty.
const APT_LISTS: &str = "/var/lib/apt/lists";

#[derive(Debug)]
pub enum LinuxManager {
    Apt,
    Dnf,
    Yum,
    Pacman,
    Zypper,
    Opkg,
    Apk,
}

fn reject_exact_if(exact: bool, manager: &'static str) -> Result<(), UpkgError> {
    if exact {
        return Err(UpkgError::Unsupported(match manager {
            "apt" => "--exact is not supported by apt search",
            "dnf" => "--exact is not supported by dnf search",
            "yum" => "--exact is not supported by yum search",
            "zypper" => "--exact is not supported by zypper search",
            _ => "--exact is not supported by this backend",
        }));
    }
    Ok(())
}

fn escape_ere(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(
            c,
            '\\' | '.' | '+' | '*' | '?' | '(' | ')' | '|' | '[' | ']' | '{' | '}' | '^' | '$'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn anchor_if_exact(query: &str, exact: bool) -> String {
    if exact {
        format!("^{}$", escape_ere(query))
    } else {
        query.to_string()
    }
}

pub fn detect() -> Result<LinuxManager, UpkgError> {
    let os_release = fs::read_to_string("/etc/os-release")?;
    detect_from(&os_release, command_exists("dnf"))
}

/// The manager for an `/etc/os-release`; `has_dnf` picks dnf over yum.
fn detect_from(os_release: &str, has_dnf: bool) -> Result<LinuxManager, UpkgError> {
    if os_release.contains("ID=ubuntu")
        || os_release.contains("ID=debian")
        || os_release.contains("ID_LIKE=debian")
    {
        return Ok(LinuxManager::Apt);
    }

    if os_release.contains("ID=fedora")
        || os_release.contains("ID=rhel")
        || os_release.contains("ID=centos")
        || os_release.contains("ID_LIKE=\"rhel fedora\"")
    {
        if has_dnf {
            return Ok(LinuxManager::Dnf);
        }
        return Ok(LinuxManager::Yum);
    }

    if os_release.contains("ID=arch") || os_release.contains("ID_LIKE=arch") {
        return Ok(LinuxManager::Pacman);
    }

    if os_release.contains("ID=opensuse") || os_release.contains("ID_LIKE=suse") {
        return Ok(LinuxManager::Zypper);
    }

    if os_release.contains("ID=openwrt") || os_release.contains("ID_LIKE=openwrt") {
        return Ok(LinuxManager::Opkg);
    }

    if os_release.contains("ID=alpine") || os_release.contains("ID_LIKE=alpine") {
        return Ok(LinuxManager::Apk);
    }

    Err(UpkgError::Unsupported("unsupported Linux distribution"))
}

/// Whether apt has fetched package lists at least once.
fn has_package_lists(dir: &Path) -> bool {
    fs::read_dir(dir)
        .is_ok_and(|entries| lists_packages(entries.flatten().map(|entry| entry.file_name())))
}

/// apt names lists `<repo>_Packages`, plus `.lz4`/`.gz` when compressed.
fn lists_packages(names: impl IntoIterator<Item = OsString>) -> bool {
    names
        .into_iter()
        .any(|name| name.to_string_lossy().contains("_Packages"))
}

impl LinuxManager {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Apt => "apt",
            Self::Dnf => "dnf",
            Self::Yum => "yum",
            Self::Pacman => "pacman",
            Self::Zypper => "zypper",
            Self::Opkg => "opkg",
            Self::Apk => "apk",
        }
    }

    fn spec(&self, args: &[&str], packages: &[String]) -> CommandSpec {
        match self {
            Self::Opkg => CommandSpec::with_packages("opkg", args, packages),
            _ => CommandSpec::with_packages("sudo", &[&[self.name()], args].concat(), packages),
        }
    }

    pub fn install_spec(&self, packages: &[String]) -> CommandSpec {
        let args: &[&str] = match self {
            Self::Apt | Self::Dnf | Self::Yum => &["install", "-y"],
            Self::Pacman => &["-S", "--noconfirm"],
            Self::Zypper => &["--non-interactive", "install"],
            Self::Opkg => &["install"],
            // Alpine images ship without an index; fetch it on every install.
            Self::Apk => &["add", "--update-cache"],
        };
        self.spec(args, packages)
    }

    /// `apt update` when no package lists were ever fetched. Other managers
    /// refresh as part of the install (apk) or need no index.
    pub fn refresh_spec(&self) -> Option<CommandSpec> {
        match self {
            Self::Apt if !has_package_lists(Path::new(APT_LISTS)) => {
                Some(self.spec(&["update"], &[]))
            }
            _ => None,
        }
    }

    pub fn uninstall_spec(&self, packages: &[String]) -> CommandSpec {
        let args: &[&str] = match self {
            Self::Apt | Self::Dnf | Self::Yum => &["remove", "-y"],
            Self::Pacman => &["-R", "--noconfirm"],
            Self::Zypper => &["--non-interactive", "remove"],
            Self::Opkg => &["remove"],
            Self::Apk => &["del"],
        };
        self.spec(args, packages)
    }

    pub fn reinstall_spec(&self, packages: &[String]) -> CommandSpec {
        let args: &[&str] = match self {
            Self::Apt => &["install", "--reinstall", "-y"],
            Self::Dnf | Self::Yum => &["reinstall", "-y"],
            // -S reinstalls packages that are already up to date.
            Self::Pacman => &["-S", "--noconfirm"],
            Self::Zypper => &["--non-interactive", "install", "--force"],
            Self::Opkg => &["install", "--force-reinstall"],
            Self::Apk => &["fix", "--reinstall"],
        };
        self.spec(args, packages)
    }

    pub fn upgrade_spec(&self, packages: &[String]) -> CommandSpec {
        let args: &[&str] = match self {
            Self::Apt if packages.is_empty() => &["upgrade", "-y"],
            Self::Apt => &["install", "--only-upgrade", "-y"],
            Self::Dnf => &["upgrade", "-y"],
            Self::Yum => &["update", "-y"],
            Self::Pacman if packages.is_empty() => &["-Syu", "--noconfirm"],
            Self::Pacman => &["-S", "--noconfirm"],
            Self::Zypper => &["--non-interactive", "update"],
            Self::Opkg => &["upgrade"],
            Self::Apk if packages.is_empty() => &["upgrade", "--update-cache"],
            Self::Apk => &["add", "--update-cache", "--upgrade"],
        };
        self.spec(args, packages)
    }

    pub fn search_spec(&self, query: &str, exact: bool) -> Result<CommandSpec, UpkgError> {
        let args = match self {
            Self::Pacman => vec!["-Ss".to_string(), anchor_if_exact(query, exact)],
            Self::Opkg => vec!["find".to_string(), anchor_if_exact(query, exact)],
            Self::Apk if exact => vec![
                "search".to_string(),
                "--exact".to_string(),
                query.to_string(),
            ],
            _ => {
                reject_exact_if(exact, self.name())?;
                vec!["search".to_string(), query.to_string()]
            }
        };
        Ok(CommandSpec::new(self.name(), args))
    }

    pub fn list_spec(&self) -> CommandSpec {
        match self {
            Self::Apt => CommandSpec::new("dpkg", vec!["--get-selections".into()]),
            Self::Dnf => CommandSpec::new("dnf", vec!["list".into(), "--installed".into()]),
            Self::Yum => CommandSpec::new("yum", vec!["list".into(), "installed".into()]),
            Self::Pacman => CommandSpec::new("pacman", vec!["-Q".into()]),
            Self::Zypper => {
                CommandSpec::new("zypper", vec!["se".into(), "--installed-only".into()])
            }
            Self::Opkg => CommandSpec::new("opkg", vec!["list-installed".into()]),
            Self::Apk => CommandSpec::new("apk", vec!["info".into()]),
        }
    }
}

#[cfg(test)]
mod tests {
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
}
