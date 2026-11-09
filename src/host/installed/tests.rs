use rstest::rstest;

use super::*;

const fn linux(manager: LinuxManager) -> Platform {
    Platform::Linux {
        manager,
        rolling: false,
    }
}

fn exited(code: i32, stdout: &str) -> Execution {
    Execution {
        code: Some(code),
        stdout: stdout.to_string(),
        stderr: String::new(),
    }
}

#[rstest]
#[case(
    linux(LinuxManager::Apt),
    "dpkg-query --show --showformat=${db:Status-Abbrev}\\n git",
    Answer::Dpkg
)]
#[case(linux(LinuxManager::Dnf), "rpm -q git", Answer::ExitStatus)]
#[case(linux(LinuxManager::Yum), "rpm -q git", Answer::ExitStatus)]
#[case(linux(LinuxManager::Zypper), "rpm -q git", Answer::ExitStatus)]
#[case(linux(LinuxManager::Pacman), "pacman -Q git", Answer::ExitStatus)]
#[case(linux(LinuxManager::Opkg), "opkg status git", Answer::Opkg)]
#[case(linux(LinuxManager::Apk), "apk info -e git", Answer::ExitStatus)]
#[case(Platform::FreeBsd { pkgbase: false }, "ASSUME_ALWAYS_YES=no pkg info -e git", Answer::ExitStatus)]
#[case(
    Platform::DragonFly,
    "ASSUME_ALWAYS_YES=no pkg info -e git",
    Answer::ExitStatus
)]
fn every_query_is_read_only_and_never_escalated(
    #[case] platform: Platform,
    #[case] rendered: &str,
    #[case] answer: Answer,
) {
    let (spec, asked) = query(platform, "git");
    assert_eq!(spec.render(), rendered);
    assert_ne!(spec.program(), "sudo");
    assert_ne!(spec.program(), "doas");
    assert_eq!(asked, answer);
}

#[test]
fn exit_status_is_the_answer() {
    assert!(Answer::ExitStatus.installed(&exited(0, "")));
    assert!(!Answer::ExitStatus.installed(&exited(1, "")));
    // pkg and apk echo the name when installed; only the status counts.
    assert!(!Answer::ExitStatus.installed(&exited(1, "git\n")));
    assert!(!Answer::ExitStatus.installed(&Execution::default()));
}

#[rstest]
#[case(0, "ii \n", true)]
#[case(0, "hi \n", true)]
// Removed, config files left behind.
#[case(0, "rc \n", false)]
// Several architectures, one installed.
#[case(0, "un \nii \n", true)]
#[case(1, "", false)]
fn dpkg_counts_the_second_status_letter(
    #[case] code: i32,
    #[case] stdout: &str,
    #[case] installed: bool,
) {
    assert_eq!(Answer::Dpkg.installed(&exited(code, stdout)), installed);
}

#[rstest]
#[case("Package: git\nStatus: install user installed\n", true)]
#[case("Package: git\nStatus: install user not-installed\n", false)]
#[case("", false)]
fn opkg_reads_the_status_line(#[case] stdout: &str, #[case] installed: bool) {
    assert_eq!(Answer::Opkg.installed(&exited(0, stdout)), installed);
}
