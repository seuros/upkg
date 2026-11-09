use super::*;
use crate::host::test_support::{Fake, exit, linux};
use rstest::rstest;

fn running(kernel: &str) -> Fake {
    Fake::default().answer("uname -r", exit(0, &format!("{kernel}\n")))
}

/// 2026-11-08T03:00:00Z
const SINCE: u64 = 1_794_106_800;

#[test]
fn json_shape_is_the_contract() {
    let status = RebootStatus {
        required: true,
        reason: Some("r".into()),
        packages: vec!["linux-image-amd64".into()],
        running_kernel: "6.1.0-25-amd64".into(),
        installed_kernel: None,
        since: Some("2026-11-08T03:00:00Z".into()),
    };
    assert_eq!(
        serde_json::to_string(&status).unwrap(),
        r#"{"required":true,"reason":"r","packages":["linux-image-amd64"],"running_kernel":"6.1.0-25-amd64","installed_kernel":null,"since":"2026-11-08T03:00:00Z"}"#
    );
    let back: RebootStatus =
        serde_json::from_str(&serde_json::to_string(&status).unwrap()).unwrap();
    assert_eq!(back, status);
}

#[test]
fn summary_is_one_line() {
    let status = RebootStatus {
        required: true,
        reason: Some("/var/run/reboot-required exists".into()),
        packages: vec!["linux-image-amd64".into(), "libc6".into()],
        running_kernel: "6.1.0-25-amd64".into(),
        installed_kernel: None,
        since: Some("2026-11-08T03:00:00Z".into()),
    };
    assert_eq!(
        status.summary(),
        "reboot required: /var/run/reboot-required exists; packages: linux-image-amd64, libc6; since 2026-11-08T03:00:00Z"
    );
    let calm = RebootStatus::not_required("6.1.0-26-amd64".into());
    assert_eq!(
        calm.summary(),
        "no reboot required (running 6.1.0-26-amd64)"
    );
    assert!(!calm.summary().contains('\n'));
}

#[test]
fn uname_failure_fails_the_check() {
    let fake = Fake::default().answer("uname -r", exit(1, ""));
    let result = check(linux(LinuxManager::Apt), &fake);
    assert!(matches!(result, Err(Error::Probe(_))), "{result:?}");
}

#[test]
fn debian_reads_the_flag_file() {
    let fake = running("6.1.0-25-amd64")
        .file(DEBIAN_FLAG, "*** System restart required ***\n", SINCE)
        .file(
            DEBIAN_PACKAGES,
            "linux-image-6.1.0-26-amd64\nlibc6\nlibc6\n",
            SINCE,
        )
        .dir("/lib/modules", &["6.1.0-25-amd64", "6.1.0-26-amd64"]);
    let status = check(linux(LinuxManager::Apt), &fake).unwrap();
    assert!(status.required);
    assert_eq!(status.packages, ["linux-image-6.1.0-26-amd64", "libc6"]);
    assert_eq!(status.since.as_deref(), Some("2026-11-08T03:00:00Z"));
    assert_eq!(status.installed_kernel.as_deref(), Some("6.1.0-26-amd64"));
    assert_eq!(status.running_kernel, "6.1.0-25-amd64");
}

#[test]
fn debian_without_the_flag_needs_nothing() {
    let fake = running("6.1.0-26-amd64").dir("/usr/lib/modules", &["6.1.0-26-amd64"]);
    let status = check(linux(LinuxManager::Apt), &fake).unwrap();
    assert!(!status.required);
    assert_eq!(status.reason, None);
    assert_eq!(status.since, None);
}

#[test]
fn needs_restarting_exit_one_with_the_message_requires_a_reboot() {
    let fake = running("6.11.4-301.fc41.x86_64")
        .command("needs-restarting")
        .answer(
            "LC_ALL=C needs-restarting -r",
            exit(1, "Core libraries or services have been updated since boot-up:\n  * kernel\n  * systemd\n\nReboot is required to fully utilize these updates.\n"),
        )
        .dir("/usr/lib/modules", &["6.11.4-301.fc41.x86_64", "6.11.5-300.fc41.x86_64"]);
    let status = check(linux(LinuxManager::Dnf), &fake).unwrap();
    assert!(status.required);
    assert_eq!(status.packages, ["kernel", "systemd"]);
    assert_eq!(
        status.installed_kernel.as_deref(),
        Some("6.11.5-300.fc41.x86_64")
    );
}

#[test]
fn dnf5_answers_through_dnf() {
    let fake = running("6.11.5-300.fc41.x86_64")
        .answer(
            "LC_ALL=C dnf needs-restarting -r",
            exit(0, "Reboot should not be necessary.\n"),
        )
        .dir("/usr/lib/modules", &["6.11.5-300.fc41.x86_64"]);
    let status = check(linux(LinuxManager::Dnf), &fake).unwrap();
    assert!(!status.required);
    assert!(
        fake.ran()
            .contains(&"LC_ALL=C dnf needs-restarting -r".to_string())
    );
}

#[test]
fn ambiguous_needs_restarting_falls_back_to_kernels() {
    // dnf exits 1 for a missing cache or an unknown command too.
    let fake = running("6.11.4-301.fc41.x86_64")
        .answer(
            "LC_ALL=C dnf needs-restarting -r",
            exit(
                1,
                "Cache-only enabled but no cache for repository \"fedora\"\n",
            ),
        )
        .dir(
            "/usr/lib/modules",
            &["6.11.4-301.fc41.x86_64", "6.11.5-300.fc41.x86_64"],
        )
        .file("/usr/lib/modules/6.11.5-300.fc41.x86_64", "", SINCE);
    let status = check(linux(LinuxManager::Dnf), &fake).unwrap();
    assert!(status.required);
    assert_eq!(
        status.reason.as_deref(),
        Some("kernel 6.11.5-300.fc41.x86_64 is installed but 6.11.4-301.fc41.x86_64 is running")
    );
    assert_eq!(status.since.as_deref(), Some("2026-11-08T03:00:00Z"));
}

#[test]
fn arch_requires_a_reboot_when_the_running_modules_are_gone() {
    let fake = running("6.11.4-arch1-1").dir("/usr/lib/modules", &["6.11.5-arch1-1"]);
    let status = check(linux(LinuxManager::Pacman), &fake).unwrap();
    assert!(status.required);
    assert!(
        status
            .reason
            .unwrap()
            .contains("6.11.4-arch1-1 has no modules under /usr/lib/modules")
    );
}

#[test]
fn arch_ignores_a_newer_kernel_of_another_package() {
    // linux-lts running, linux installed alongside: both trees present.
    let fake = running("6.6.56-1-lts").dir("/usr/lib/modules", &["6.6.56-1-lts", "6.11.5-arch1-1"]);
    let status = check(linux(LinuxManager::Pacman), &fake).unwrap();
    assert!(!status.required, "{status:?}");
}

#[test]
fn alpine_compares_kernels_of_the_same_flavor() {
    let fake = running("6.6.30-r0-lts").dir("/lib/modules", &["6.6.31-r0-lts", "6.6.40-r0-virt"]);
    let status = check(linux(LinuxManager::Apk), &fake).unwrap();
    assert!(status.required);
    assert_eq!(status.installed_kernel.as_deref(), Some("6.6.31-r0-lts"));
}

#[test]
fn no_modules_means_cannot_tell() {
    let status = check(linux(LinuxManager::Apk), &running("6.6.30-0-lts")).unwrap();
    assert!(!status.required);
    assert!(status.reason.unwrap().starts_with("cannot tell"));
    assert_eq!(status.installed_kernel, None);
}

#[test]
fn freebsd_kernel_mismatch_requires_a_reboot() {
    let fake = running("15.1-RELEASE-p2")
        .answer("freebsd-version -k", exit(0, "15.1-RELEASE-p3\n"))
        .file(FREEBSD_KERNEL, "", SINCE);
    let status = check(Platform::FreeBsd { pkgbase: false }, &fake).unwrap();
    assert!(status.required);
    assert_eq!(status.installed_kernel.as_deref(), Some("15.1-RELEASE-p3"));
    assert_eq!(status.since.as_deref(), Some("2026-11-08T03:00:00Z"));
}

#[test]
fn freebsd_pending_install_is_reported_not_required() {
    let fake =
        running("15.1-RELEASE-p3").answer("freebsd-version -k", exit(0, "15.1-RELEASE-p3\n"));
    let status = check(Platform::FreeBsd { pkgbase: false }, &fake).unwrap();
    assert!(!status.required);
    assert!(status.reason.unwrap().contains("freebsd-update install"));
}

#[test]
fn freebsd_without_pending_updates_is_calm() {
    let fake = running("15.1-RELEASE-p3")
        .answer("freebsd-version -k", exit(0, "15.1-RELEASE-p3\n"))
        .answer(
            "freebsd-update updatesready",
            exit(2, "No updates are available to install.\n"),
        );
    let status = check(Platform::FreeBsd { pkgbase: false }, &fake).unwrap();
    assert!(!status.required);
    assert_eq!(status.reason, None);
}

#[test]
fn pkgbase_skips_freebsd_update() {
    let fake =
        running("15.1-RELEASE-p3").answer("freebsd-version -k", exit(0, "15.1-RELEASE-p3\n"));
    let status = check(Platform::FreeBsd { pkgbase: true }, &fake).unwrap();
    assert_eq!(status.reason, None);
    assert!(
        !fake
            .ran()
            .iter()
            .any(|command| command.contains("freebsd-update"))
    );
}

#[test]
fn freebsd_version_failure_fails_the_check() {
    let fake = running("15.1-RELEASE-p3").missing("freebsd-version -k");
    let result = check(Platform::FreeBsd { pkgbase: false }, &fake);
    assert!(matches!(result, Err(Error::Io { .. })), "{result:?}");
}

#[test]
fn dragonfly_cannot_tell() {
    let status = check(Platform::DragonFly, &running("6.4-RELEASE")).unwrap();
    assert!(!status.required);
    assert!(status.reason.unwrap().starts_with("cannot tell"));
}

#[rstest]
#[case("6.1.0-26-amd64", "amd64")]
#[case("6.6.30-0-lts", "lts")]
#[case("6.11.4-301.fc41.x86_64", "")]
#[case("6.11.4-arch1-1", "")]
#[case("6.11.3-1-default", "default")]
fn kernel_flavors(#[case] release: &str, #[case] expected: &str) {
    assert_eq!(flavor(release), expected);
}

#[rstest]
#[case("6.10.0", "6.9.12", Ordering::Greater)]
#[case("6.1.0-26-amd64", "6.1.0-25-amd64", Ordering::Greater)]
#[case("5.15.0", "5.15.0", Ordering::Equal)]
#[case("6.6.30-r0", "6.6.30-r1", Ordering::Less)]
#[case("6.6", "6.6.1", Ordering::Less)]
fn natural_version_order(#[case] a: &str, #[case] b: &str, #[case] expected: Ordering) {
    assert_eq!(version_cmp(a, b), expected);
}

#[test]
fn package_list_drops_blanks_and_repeats() {
    assert_eq!(package_list("a\n\nb\na\n"), ["a", "b"]);
}

#[rstest]
#[case(Some(0), "", Some(false))]
#[case(
    Some(1),
    "Reboot is required to fully utilize these updates.",
    Some(true)
)]
#[case(Some(1), "No such command: needs-restarting.", None)]
#[case(Some(2), "Unknown argument", None)]
#[case(None, "", None)]
fn needs_restarting_answers(
    #[case] code: Option<i32>,
    #[case] stdout: &str,
    #[case] expected: Option<bool>,
) {
    assert_eq!(
        restart_verdict(code, stdout).map(|restart| restart.required),
        expected
    );
}

#[rstest]
#[case(0, "1970-01-01T00:00:00Z")]
#[case(SINCE, "2026-11-08T03:00:00Z")]
#[case(951_782_400, "2000-02-29T00:00:00Z")]
#[case(4_107_542_399, "2100-02-28T23:59:59Z")]
fn rfc3339_formats_utc(#[case] seconds: u64, #[case] expected: &str) {
    assert_eq!(
        rfc3339(UNIX_EPOCH + std::time::Duration::from_secs(seconds)),
        expected
    );
}
