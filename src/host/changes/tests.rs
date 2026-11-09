use super::*;
use rstest::rstest;

const fn changes(upgraded: u32, installed: u32, removed: u32) -> Changes {
    Changes {
        upgraded,
        installed,
        removed,
    }
}

#[rstest]
#[case::apt_upgrade(
    Counter::Apt,
    "Reading state information...\n1 upgraded, 2 newly installed, 0 to remove and 0 not upgraded.\nInst zlib1g\n",
    Some(changes(1, 2, 0))
)]
#[case::apt_autoremove(
    Counter::Apt,
    "0 upgraded, 0 newly installed, 3 to remove and 1 not upgraded.\n",
    Some(changes(0, 0, 3))
)]
#[case::apt_silent(Counter::Apt, "Reading package lists...\n", None)]
#[case::dnf5(
    Counter::Dnf,
    "Transaction Summary:\n Installing:         1 package\n Upgrading:         14 packages\n Replacing:         14 packages\n",
    Some(changes(14, 1, 0))
)]
#[case::dnf4(
    Counter::Dnf,
    "Upgrading:\n kernel  x86_64 ...\nTransaction Summary\n====\nInstall   2 Packages\nUpgrade  15 Packages\nRemove    1 Package\n",
    Some(changes(15, 2, 1))
)]
#[case::dnf_nothing(
    Counter::Dnf,
    "Dependencies resolved.\nNothing to do.\nComplete!\n",
    Some(changes(0, 0, 0))
)]
#[case::pacman_upgrade(
    Counter::Pacman(Direction::Upgraded),
    ":: Starting full system upgrade...\nPackages (3) a-1  b-2  c-3\n\nTotal Download Size: 1 MiB\n",
    Some(changes(3, 0, 0))
)]
#[case::pacman_remove(
    Counter::Pacman(Direction::Removed),
    "checking dependencies...\n\nPackages (2) a-1  b-2\n",
    Some(changes(0, 0, 2))
)]
#[case::pacman_install(
    Counter::Pacman(Direction::Installed),
    "resolving dependencies...\n\nPackages (2) pkgconf-2.4  protobuf-30.2\n",
    Some(changes(0, 2, 0))
)]
#[case::opkg_unknown(
    Counter::Unknown,
    "Installing protobuf (3.21) to root...\nConfiguring protobuf.\n",
    None
)]
#[case::pacman_nothing(
    Counter::Pacman(Direction::Upgraded),
    ":: Starting full system upgrade...\n there is nothing to do\n",
    Some(changes(0, 0, 0))
)]
#[case::zypper(
    Counter::Zypper,
    "The following 5 packages are going to be upgraded:\n  a b\n\nThe following NEW package is going to be installed:\n  c\n\nThe following package is going to be REMOVED:\n  d\n",
    Some(changes(5, 1, 1))
)]
#[case::zypper_nothing(
    Counter::Zypper,
    "Loading repository data...\nNothing to do.\n",
    Some(changes(0, 0, 0))
)]
#[case::apk(
    Counter::Apk,
    "(1/3) Upgrading zlib (1.3.2-r1 -> 1.4-r0)\n(2/3) Installing foo (1.0-r0)\n(3/3) Purging bar (2.0-r0)\nOK: 8432 KiB in 16 packages\n",
    Some(changes(1, 1, 1))
)]
#[case::apk_nothing(Counter::Apk, "OK: 8432 KiB in 16 packages\n", Some(changes(0, 0, 0)))]
#[case::pkg_upgrade(
    Counter::Pkg,
    "Checking for upgrades (3 candidates): 100%\nNumber of packages to be installed: 1\nNumber of packages to be upgraded: 2\n",
    Some(changes(2, 1, 0))
)]
#[case::pkg_up_to_date(
    Counter::Pkg,
    "Updating FreeBSD repository catalogue...\nChecking for upgrades (0 candidates): 100%\nYour packages are up to date.\n",
    Some(changes(0, 0, 0))
)]
#[case::pkg_autoremove(
    Counter::Pkg,
    "Installed packages to be REMOVED:\n\tfoo: 1.0\n\nNumber of packages to be removed: 1\n",
    Some(changes(0, 0, 1))
)]
#[case::pkg_autoremove_nothing(
    Counter::Pkg,
    "Checking integrity... done (0 conflicting)\nNothing to do.\n",
    Some(changes(0, 0, 0))
)]
#[case::metadata(
    Counter::Nothing,
    "Hit:1 http://deb.debian.org trixie InRelease\n",
    Some(changes(0, 0, 0))
)]
fn counts_packages(
    #[case] counter: Counter,
    #[case] stdout: &str,
    #[case] expected: Option<Changes>,
) {
    let (changed, counted) = count(counter, stdout, false);
    assert_eq!(counted, expected, "{stdout}");
    assert_eq!(changed, expected.map(|changes| !changes.is_empty()));
}

#[test]
fn freebsd_update_install_changes_base_unless_settled() {
    let installed = count(
        Counter::FreebsdUpdate,
        "Installing updates... done.\n",
        false,
    );
    assert_eq!(installed, (Some(true), None));

    let nothing = count(
        Counter::FreebsdUpdate,
        "No updates are available to install.\n",
        true,
    );
    assert_eq!(nothing, (Some(false), Some(Changes::default())));
}

#[test]
fn changes_add_up() {
    assert_eq!(changes(1, 2, 3).plus(changes(4, 5, 6)), changes(5, 7, 9));
    assert!(Changes::default().is_empty());
}
