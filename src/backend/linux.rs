use std::fs;

use crate::backend::{CommandSpec, command_exists};
use crate::error::UpkgError;

pub enum LinuxManager {
    Apt,
    Dnf,
    Yum,
    Pacman,
    Zypper,
    Opkg,
}

pub fn detect() -> Result<LinuxManager, UpkgError> {
    let os_release = fs::read_to_string("/etc/os-release")?;

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
        if command_exists("dnf") {
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

    Err(UpkgError::Unsupported("unsupported Linux distribution"))
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
        }
    }

    pub fn install_spec(&self, packages: &[String]) -> CommandSpec {
        let mut args: Vec<String> = match self {
            Self::Apt => vec!["apt".into(), "install".into(), "-y".into()],
            Self::Dnf => vec!["dnf".into(), "install".into(), "-y".into()],
            Self::Yum => vec!["yum".into(), "install".into(), "-y".into()],
            Self::Pacman => vec!["pacman".into(), "-S".into(), "--noconfirm".into()],
            Self::Zypper => vec![
                "zypper".into(),
                "--non-interactive".into(),
                "install".into(),
            ],
            Self::Opkg => vec!["install".into()],
        };
        args.extend(packages.iter().cloned());

        match self {
            Self::Opkg => CommandSpec::new("opkg", args),
            _ => CommandSpec::new("sudo", args),
        }
    }

    pub fn uninstall_spec(&self, packages: &[String]) -> CommandSpec {
        let mut args: Vec<String> = match self {
            Self::Apt => vec!["apt".into(), "remove".into(), "-y".into()],
            Self::Dnf => vec!["dnf".into(), "remove".into(), "-y".into()],
            Self::Yum => vec!["yum".into(), "remove".into(), "-y".into()],
            Self::Pacman => vec!["pacman".into(), "-R".into(), "--noconfirm".into()],
            Self::Zypper => vec!["zypper".into(), "--non-interactive".into(), "remove".into()],
            Self::Opkg => vec!["remove".into()],
        };
        args.extend(packages.iter().cloned());

        match self {
            Self::Opkg => CommandSpec::new("opkg", args),
            _ => CommandSpec::new("sudo", args),
        }
    }

    pub fn upgrade_spec(&self, packages: &[String]) -> CommandSpec {
        let mut args: Vec<String> = match self {
            Self::Apt => {
                if packages.is_empty() {
                    vec!["apt".into(), "upgrade".into(), "-y".into()]
                } else {
                    vec![
                        "apt".into(),
                        "install".into(),
                        "--only-upgrade".into(),
                        "-y".into(),
                    ]
                }
            }
            Self::Dnf => vec!["dnf".into(), "upgrade".into(), "-y".into()],
            Self::Yum => vec!["yum".into(), "update".into(), "-y".into()],
            Self::Pacman => {
                if packages.is_empty() {
                    vec!["pacman".into(), "-Syu".into(), "--noconfirm".into()]
                } else {
                    vec!["pacman".into(), "-S".into(), "--noconfirm".into()]
                }
            }
            Self::Zypper => vec!["zypper".into(), "--non-interactive".into(), "update".into()],
            Self::Opkg => vec!["upgrade".into()],
        };
        args.extend(packages.iter().cloned());

        match self {
            Self::Opkg => CommandSpec::new("opkg", args),
            _ => CommandSpec::new("sudo", args),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    // Test helper that parses content directly instead of reading /etc/os-release
    fn detect_with_os_release(content: &str) -> Result<LinuxManager, UpkgError> {
        if content.contains("ID=ubuntu")
            || content.contains("ID=debian")
            || content.contains("ID_LIKE=debian")
        {
            return Ok(LinuxManager::Apt);
        }

        if content.contains("ID=fedora")
            || content.contains("ID=rhel")
            || content.contains("ID=centos")
            || content.contains("ID_LIKE=\"rhel fedora\"")
        {
            // In tests, assume dnf is available for simplicity
            return Ok(LinuxManager::Dnf);
        }

        if content.contains("ID=arch") || content.contains("ID_LIKE=arch") {
            return Ok(LinuxManager::Pacman);
        }

        if content.contains("ID=opensuse") || content.contains("ID_LIKE=suse") {
            return Ok(LinuxManager::Zypper);
        }

        if content.contains("ID=openwrt") || content.contains("ID_LIKE=openwrt") {
            return Ok(LinuxManager::Opkg);
        }

        Err(UpkgError::Unsupported("unsupported Linux distribution"))
    }

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
    fn detect_distro_from_os_release(
        #[case] os_release: &str,
        #[case] expected_manager: &str,
        #[case] _description: &str,
    ) {
        let manager = detect_with_os_release(os_release).unwrap();
        assert_eq!(manager.name(), expected_manager);
    }

    #[test]
    fn detect_unsupported_distro() {
        let result = detect_with_os_release("ID=unknown\n");
        assert!(result.is_err());
        assert!(matches!(result, Err(UpkgError::Unsupported(_))));
    }

    #[rstest]
    #[case(LinuxManager::Apt, vec!["git".into()], "sudo", vec!["apt", "install", "-y", "git"])]
    #[case(LinuxManager::Dnf, vec!["curl".into()], "sudo", vec!["dnf", "install", "-y", "curl"])]
    #[case(LinuxManager::Yum, vec!["wget".into()], "sudo", vec!["yum", "install", "-y", "wget"])]
    #[case(LinuxManager::Pacman, vec!["vim".into()], "sudo", vec!["pacman", "-S", "--noconfirm", "vim"])]
    #[case(LinuxManager::Zypper, vec!["gcc".into()], "sudo", vec!["zypper", "--non-interactive", "install", "gcc"])]
    #[case(LinuxManager::Opkg, vec!["ca-certificates".into()], "opkg", vec!["install", "ca-certificates"])]
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

    #[test]
    fn upgrade_spec_handles_empty_packages_for_apt() {
        let manager = LinuxManager::Apt;
        let packages: Vec<String> = vec![];
        let spec = manager.upgrade_spec(&packages);

        let args: Vec<&str> = spec.args().iter().map(|s| s.as_str()).collect();
        assert_eq!(args, vec!["apt", "upgrade", "-y"]);
    }
}
