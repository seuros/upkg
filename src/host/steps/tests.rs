use super::*;
use crate::command::Privilege;
use rstest::rstest;

const fn linux(manager: LinuxManager) -> Platform {
    Platform::Linux {
        manager,
        rolling: false,
    }
}

/// Each step as a line: the rendered command, `note: ...`, or the query and
/// removal of a listed-removal step.
fn lines(platform: Platform, task: Task) -> Vec<String> {
    plan(platform, task)
        .unwrap()
        .into_iter()
        .flat_map(|step| match step {
            Step::Run { spec, .. } | Step::Settle { spec, .. } => vec![spec.render()],
            Step::RemoveListed { query, remove, .. } => {
                vec![query.render(), format!("{} <listed>", remove.render())]
            }
            Step::Note(text) => vec![format!("note: {text}")],
        })
        .collect()
}

const APT_ENV: &str = "sudo DEBIAN_FRONTEND=noninteractive apt-get -o Dpkg::Options::=--force-confdef -o Dpkg::Options::=--force-confold";

#[rstest]
#[case(LinuxManager::Apt, &["sudo apt-get update"])]
#[case(LinuxManager::Dnf, &["sudo dnf makecache"])]
#[case(LinuxManager::Yum, &["sudo yum makecache"])]
#[case(LinuxManager::Zypper, &["sudo zypper --non-interactive refresh"])]
#[case(LinuxManager::Apk, &["sudo apk update"])]
#[case(LinuxManager::Opkg, &["opkg update"])]
fn refresh_fetches_metadata_only(#[case] manager: LinuxManager, #[case] expected: &[&str]) {
    assert_eq!(lines(linux(manager), Task::Refresh), expected);
}

#[test]
fn pacman_refresh_is_a_note() {
    let lines = lines(linux(LinuxManager::Pacman), Task::Refresh);
    assert_eq!(lines.len(), 1);
    assert!(
        lines[0].starts_with("note: pacman has no safe metadata-only refresh"),
        "{lines:?}"
    );
}

#[rstest]
#[case(LinuxManager::Dnf, "sudo dnf upgrade -y")]
#[case(LinuxManager::Yum, "sudo yum upgrade -y")]
#[case(LinuxManager::Pacman, "sudo pacman -Syu --noconfirm")]
#[case(LinuxManager::Zypper, "sudo zypper --non-interactive update")]
#[case(LinuxManager::Apk, "sudo apk upgrade --update-cache")]
fn full_upgrade_upgrades_everything(#[case] manager: LinuxManager, #[case] expected: &str) {
    assert_eq!(lines(linux(manager), Task::FullUpgrade), [expected]);
}

#[test]
fn apt_full_upgrade_never_prompts() {
    assert_eq!(
        lines(linux(LinuxManager::Apt), Task::FullUpgrade),
        [format!("{APT_ENV} full-upgrade -y")]
    );
}

#[test]
fn tumbleweed_full_upgrade_is_a_dist_upgrade() {
    let tumbleweed = Platform::Linux {
        manager: LinuxManager::Zypper,
        rolling: true,
    };
    assert_eq!(
        lines(tumbleweed, Task::FullUpgrade),
        ["sudo zypper --non-interactive dist-upgrade"]
    );
}

#[test]
fn opkg_full_upgrade_is_unsupported() {
    let result = plan(linux(LinuxManager::Opkg), Task::FullUpgrade);
    assert!(matches!(result, Err(Error::Unsupported(_))), "{result:?}");
}

#[test]
fn freebsd_full_upgrade_covers_packages_then_base() {
    assert_eq!(
        lines(Platform::FreeBsd { pkgbase: false }, Task::FullUpgrade),
        [
            "sudo pkg upgrade -y",
            "sudo PAGER=cat freebsd-update fetch --not-running-from-cron",
            "sudo PAGER=cat freebsd-update install",
        ]
    );
}

#[test]
fn freebsd_update_settles_on_nothing_to_do() {
    let steps = plan(Platform::FreeBsd { pkgbase: false }, Task::FullUpgrade).unwrap();
    for step in &steps[1..] {
        let Step::Settle { settled_by, .. } = step else {
            panic!("freebsd-update must settle: {step:?}");
        };
        assert!(settled_by.contains(&"No updates needed"));
        assert!(settled_by.contains(&"No updates are available to install"));
    }
}

#[test]
fn pkgbase_full_upgrade_leaves_freebsd_update_out() {
    assert_eq!(
        lines(Platform::FreeBsd { pkgbase: true }, Task::FullUpgrade),
        ["sudo pkg upgrade -y"]
    );
}

#[test]
fn dragonfly_full_upgrade_notes_the_base() {
    let lines = lines(Platform::DragonFly, Task::FullUpgrade);
    assert_eq!(lines[0], "sudo pkg upgrade -y");
    assert!(lines[1].starts_with("note: DragonFly's base"), "{lines:?}");
}

#[rstest]
#[case(Platform::FreeBsd { pkgbase: false })]
#[case(Platform::FreeBsd { pkgbase: true })]
#[case(Platform::DragonFly)]
fn bsd_refresh_and_cleanup(#[case] platform: Platform) {
    assert_eq!(lines(platform, Task::Refresh), ["sudo pkg update"]);
    assert_eq!(
        lines(platform, Task::Cleanup),
        ["sudo pkg autoremove -y", "sudo pkg clean -ay"]
    );
}

#[test]
fn apt_cleanup_purges_orphans_then_cleans() {
    assert_eq!(
        lines(linux(LinuxManager::Apt), Task::Cleanup),
        [
            format!("{APT_ENV} autoremove --purge -y"),
            "sudo apt-get clean".to_string(),
        ]
    );
}

#[rstest]
#[case(LinuxManager::Dnf, &["sudo dnf autoremove -y", "sudo dnf clean all"])]
#[case(LinuxManager::Yum, &["sudo yum autoremove -y", "sudo yum clean all"])]
#[case(LinuxManager::Pacman, &["pacman -Qdtq", "sudo pacman -Rns --noconfirm <listed>", "sudo pacman -Sc --noconfirm"])]
#[case(LinuxManager::Apk, &["sudo apk cache clean"])]
fn cleanup(#[case] manager: LinuxManager, #[case] expected: &[&str]) {
    assert_eq!(lines(linux(manager), Task::Cleanup), expected);
}

#[test]
fn zypper_cleanup_cleans_and_says_why_orphans_stay() {
    let lines = lines(linux(LinuxManager::Zypper), Task::Cleanup);
    assert!(
        lines[0].starts_with("note: zypper has no autoremove"),
        "{lines:?}"
    );
    assert_eq!(lines[1], "sudo zypper --non-interactive clean");
}

#[test]
fn opkg_cleanup_is_a_note() {
    let lines = lines(linux(LinuxManager::Opkg), Task::Cleanup);
    assert_eq!(lines.len(), 1);
    assert!(lines[0].starts_with("note: opkg"), "{lines:?}");
}

#[test]
fn every_privileged_step_escalates_through_privilege() {
    let platforms = [
        linux(LinuxManager::Apt),
        linux(LinuxManager::Pacman),
        Platform::FreeBsd { pkgbase: false },
    ];
    for platform in platforms {
        for task in [Task::Refresh, Task::FullUpgrade, Task::Cleanup] {
            for step in plan(platform, task).unwrap() {
                let spec = match step {
                    Step::Run { spec, .. } | Step::Settle { spec, .. } => spec,
                    Step::RemoveListed { remove, .. } => remove,
                    Step::Note(_) => continue,
                };
                let rendered = spec.escalate(Privilege::AlreadyRoot).render();
                assert!(!rendered.contains("sudo"), "{rendered}");
            }
        }
    }
}

fn install_lines(platform: Platform, fetch_index: bool) -> Vec<String> {
    let packages = ["pkgconf".to_string(), "protobuf".to_string()];
    install(platform, &packages, fetch_index)
        .into_iter()
        .map(|step| match step {
            Step::Run { spec, .. } => spec.render(),
            other => panic!("install plans only commands, got {other:?}"),
        })
        .collect()
}

#[rstest]
#[case(LinuxManager::Dnf, "sudo dnf install -y pkgconf protobuf")]
#[case(LinuxManager::Yum, "sudo yum install -y pkgconf protobuf")]
#[case(
    LinuxManager::Pacman,
    "sudo pacman -S --needed --noconfirm pkgconf protobuf"
)]
#[case(
    LinuxManager::Zypper,
    "sudo zypper --non-interactive install pkgconf protobuf"
)]
#[case(LinuxManager::Apk, "sudo apk add --update-cache pkgconf protobuf")]
#[case(LinuxManager::Opkg, "opkg install pkgconf protobuf")]
fn install_is_one_unattended_command(#[case] manager: LinuxManager, #[case] expected: &str) {
    assert_eq!(install_lines(linux(manager), false), [expected]);
}

#[test]
fn apt_installs_unattended_and_fetches_a_missing_index() {
    let install = format!("{APT_ENV} install -y pkgconf protobuf");
    assert_eq!(
        install_lines(linux(LinuxManager::Apt), false),
        std::slice::from_ref(&install)
    );
    assert_eq!(
        install_lines(linux(LinuxManager::Apt), true),
        ["sudo apt-get update".to_string(), install]
    );
}

#[rstest]
#[case(Platform::FreeBsd { pkgbase: false })]
#[case(Platform::FreeBsd { pkgbase: true })]
#[case(Platform::DragonFly)]
fn pkg_installs_without_asking_even_to_bootstrap(#[case] platform: Platform) {
    assert_eq!(
        install_lines(platform, false),
        ["sudo ASSUME_ALWAYS_YES=yes pkg install -y pkgconf protobuf"]
    );
}

#[test]
fn install_task_has_no_plan_without_packages() {
    for platform in [linux(LinuxManager::Apt), Platform::DragonFly] {
        let Err(Error::Unsupported(message)) = plan(platform, Task::Install) else {
            panic!("Task::Install must be refused on {platform:?}");
        };
        assert!(message.contains("Host::install"), "{message}");
    }
}
