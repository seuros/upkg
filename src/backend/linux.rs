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

fn anchor_if_exact(query: &str, exact: bool) -> String {
    if exact {
        format!("^{}$", regex::escape(query))
    } else {
        query.to_string()
    }
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

    pub fn search_spec(&self, query: &str, exact: bool) -> Result<CommandSpec, UpkgError> {
        match self {
            Self::Apt => {
                reject_exact_if(exact, "apt")?;
                Ok(CommandSpec::new(
                    "apt",
                    vec!["search".into(), query.to_string()],
                ))
            }
            Self::Dnf => {
                reject_exact_if(exact, "dnf")?;
                Ok(CommandSpec::new(
                    "dnf",
                    vec!["search".into(), query.to_string()],
                ))
            }
            Self::Yum => {
                reject_exact_if(exact, "yum")?;
                Ok(CommandSpec::new(
                    "yum",
                    vec!["search".into(), query.to_string()],
                ))
            }
            Self::Zypper => {
                reject_exact_if(exact, "zypper")?;
                Ok(CommandSpec::new(
                    "zypper",
                    vec!["search".into(), query.to_string()],
                ))
            }
            Self::Pacman => Ok(CommandSpec::new(
                "pacman",
                vec!["-Ss".into(), anchor_if_exact(query, exact)],
            )),
            Self::Opkg => Ok(CommandSpec::new(
                "opkg",
                vec!["find".into(), anchor_if_exact(query, exact)],
            )),
        }
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

    #[rstest]
    #[case(LinuxManager::Apt, "ripgrep", "apt", vec!["search", "ripgrep"])]
    #[case(LinuxManager::Dnf, "ripgrep", "dnf", vec!["search", "ripgrep"])]
    #[case(LinuxManager::Yum, "ripgrep", "yum", vec!["search", "ripgrep"])]
    #[case(LinuxManager::Zypper, "ripgrep", "zypper", vec!["search", "ripgrep"])]
    #[case(LinuxManager::Pacman, "ripgrep", "pacman", vec!["-Ss", "ripgrep"])]
    #[case(LinuxManager::Opkg, "ripgrep", "opkg", vec!["find", "ripgrep"])]
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
        assert!(matches!(err, UpkgError::Unsupported(_)));
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
}
