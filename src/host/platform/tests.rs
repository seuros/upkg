use super::*;
use crate::host::Execution;
use rstest::rstest;
use std::io;

#[rstest]
#[case("ID=ubuntu\n", "apt")]
#[case("ID=debian\n", "apt")]
#[case("ID=mint\nID_LIKE=debian\n", "apt")]
#[case("ID=pop\nID_LIKE=\"ubuntu debian\"\n", "apt")]
#[case("ID=fedora\n", "dnf")]
#[case("ID=rhel\n", "dnf")]
#[case("ID=centos\n", "dnf")]
#[case("ID=\"rocky\"\nID_LIKE=\"rhel centos fedora\"\n", "dnf")]
#[case("ID=arch\n", "pacman")]
#[case("ID=manjaro\nID_LIKE=arch\n", "pacman")]
#[case("ID=opensuse\n", "zypper")]
#[case("ID=\"opensuse-tumbleweed\"\nID_LIKE=\"opensuse suse\"\n", "zypper")]
#[case("ID=\"opensuse-leap\"\nID_LIKE=\"suse opensuse\"\n", "zypper")]
#[case("ID=\"sles\"\nID_LIKE=\"suse\"\n", "zypper")]
#[case("ID=openwrt\n", "opkg")]
#[case("ID=alpine\n", "apk")]
#[case("ID=postmarketos\nID_LIKE=alpine\n", "apk")]
fn manager_from_os_release(#[case] os_release: &str, #[case] expected: &str) {
    let manager = LinuxManager::from_os_release(os_release, true).unwrap();
    assert_eq!(manager.name(), expected);
}

#[test]
fn version_id_is_not_an_id() {
    let result = LinuxManager::from_os_release("VERSION_ID=debian\nID=plan9\n", true);
    assert!(matches!(result, Err(Error::Unsupported(_))), "{result:?}");
}

#[test]
fn falls_back_to_yum_without_dnf() {
    let manager = LinuxManager::from_os_release("ID=centos\n", false).unwrap();
    assert_eq!(manager, LinuxManager::Yum);
}

#[test]
fn unknown_distro_is_unsupported() {
    let result = LinuxManager::from_os_release("ID=unknown\n", true);
    assert!(matches!(result, Err(Error::Unsupported(_))), "{result:?}");
}

#[rstest]
#[case("ID=\"opensuse-tumbleweed\"\nID_LIKE=\"opensuse suse\"\n", true)]
#[case("ID=\"opensuse-slowroll\"\nID_LIKE=\"opensuse suse\"\n", true)]
#[case("ID=\"opensuse-leap\"\nID_LIKE=\"suse opensuse\"\n", false)]
#[case("ID=debian\n", false)]
fn rolling_releases(#[case] os_release: &str, #[case] rolling: bool) {
    let platform = Platform::from_os_release(os_release, true).unwrap();
    assert!(
        matches!(platform, Platform::Linux { rolling: r, .. } if r == rolling),
        "{platform:?}"
    );
}

#[test]
fn bsd_platforms_use_pkg() {
    assert_eq!(Platform::FreeBsd { pkgbase: false }.manager(), "pkg");
    assert_eq!(Platform::DragonFly.manager(), "pkg");
}

struct Pkg(Option<i32>);

impl Runner for Pkg {
    fn run(&self, spec: &CommandSpec) -> io::Result<Execution> {
        assert_eq!(spec.render(), "pkg which /usr/bin/uname");
        Ok(Execution {
            code: self.0,
            ..Execution::default()
        })
    }
}

#[test]
fn pkgbase_is_uname_owned_by_a_package() {
    assert!(pkgbase(&Pkg(Some(0))));
    assert!(!pkgbase(&Pkg(Some(1))));
}
