use super::test_support::{Fake, exit};
use super::*;

const fn linux(manager: LinuxManager) -> Platform {
    Platform::Linux {
        manager,
        rolling: false,
    }
}

const FREEBSD: Platform = Platform::FreeBsd { pkgbase: false };

fn run(
    platform: Platform,
    fake: Fake,
    task: Task,
    options: Options,
) -> (Result<Outcome, Error>, Vec<String>) {
    let host = Host::new(platform, fake);
    let result = host.perform(task, &options);
    (result, host.runner.ran())
}

const ROOT: Options = Options {
    dry_run: false,
    privilege: Privilege::AlreadyRoot,
};

#[test]
fn dry_run_runs_nothing() {
    let options = Options {
        dry_run: true,
        privilege: Privilege::Sudo,
    };
    let (outcome, ran) = run(FREEBSD, Fake::default(), Task::FullUpgrade, options);
    let outcome = outcome.unwrap();
    assert_eq!(ran, Vec::<String>::new());
    assert!(outcome.dry_run);
    assert!(
        outcome
            .steps
            .iter()
            .all(|step| step.state == StepState::Planned)
    );
    assert_eq!(
        outcome.steps[0].command.as_deref(),
        Some("sudo pkg upgrade -y")
    );
    assert_eq!(outcome.changed(), None);
}

#[test]
fn root_runs_without_sudo() {
    let (outcome, ran) = run(
        linux(LinuxManager::Apt),
        Fake::default(),
        Task::Refresh,
        ROOT,
    );
    assert_eq!(ran, ["apt-get update"]);
    assert_eq!(outcome.unwrap().steps[0].state, StepState::Succeeded);
}

#[test]
fn auto_privilege_asks_the_runner() {
    let options = Options::default();
    let (_, ran) = run(
        linux(LinuxManager::Apk),
        Fake::default().root(),
        Task::Refresh,
        options,
    );
    assert_eq!(ran, ["apk update"]);

    let (_, ran) = run(
        linux(LinuxManager::Apk),
        Fake::default(),
        Task::Refresh,
        options,
    );
    assert_eq!(ran, ["sudo apk update"]);

    let doas_only = Fake::default().command("doas");
    let (outcome, _) = run(
        linux(LinuxManager::Apk),
        doas_only,
        Task::Refresh,
        Options {
            dry_run: true,
            ..options
        },
    );
    // The fake has sudo unless root; doas wins only without sudo.
    assert_eq!(
        outcome.unwrap().steps[0].command.as_deref(),
        Some("sudo apk update")
    );
}

#[test]
fn freebsd_update_with_nothing_to_do_settles() {
    let fake = Fake::default()
        .answer(
            "pkg upgrade -y",
            exit(0, "Updating FreeBSD repository catalogue...\nYour packages are up to date.\n"),
        )
        .answer(
            "PAGER=cat freebsd-update fetch --not-running-from-cron",
            exit(0, "No updates needed to update system to 15.1-RELEASE-p3.\n"),
        )
        .answer(
            "PAGER=cat freebsd-update install",
            exit(2, "No updates are available to install.\nRun 'freebsd-update [options] fetch' first.\n"),
        );
    let (outcome, ran) = run(FREEBSD, fake, Task::FullUpgrade, ROOT);
    let outcome = outcome.unwrap();
    assert_eq!(ran.len(), 3);
    let states: Vec<_> = outcome
        .steps
        .iter()
        .map(|step| step.state.clone())
        .collect();
    assert_eq!(
        states,
        [
            StepState::Succeeded,
            StepState::Succeeded,
            StepState::Settled
        ]
    );
    assert_eq!(outcome.steps[2].code, Some(2));
    assert_eq!(outcome.changed(), Some(false));
    assert_eq!(outcome.changes(), Some(Changes::default()));
}

#[test]
fn freebsd_update_install_changes_the_base() {
    let fake = Fake::default().answer(
        "PAGER=cat freebsd-update install",
        exit(0, "Installing updates...\n done.\n"),
    );
    let (outcome, _) = run(FREEBSD, fake, Task::FullUpgrade, ROOT);
    let outcome = outcome.unwrap();
    assert_eq!(outcome.changed(), Some(true));
    // pkg printed nothing countable, so no package totals.
    assert_eq!(outcome.changes(), None);
}

#[test]
fn a_failed_step_stops_the_task_and_keeps_the_outcome() {
    let fake = Fake::default().answer(
        "pkg upgrade -y",
        Execution {
            code: Some(3),
            stdout: "partial\n".into(),
            stderr: "pkg: repository unreachable\n".into(),
        },
    );
    let (result, ran) = run(FREEBSD, fake, Task::FullUpgrade, ROOT);
    assert_eq!(ran, ["pkg upgrade -y"]);
    let Err(Error::Failed {
        command,
        code,
        outcome,
    }) = result
    else {
        panic!("expected a failure: {result:?}");
    };
    assert_eq!(command, "pkg upgrade -y");
    assert_eq!(code, Some(3));
    let failed = outcome.steps.last().unwrap();
    assert_eq!(failed.state, StepState::Failed);
    assert_eq!(failed.stderr_tail, "pkg: repository unreachable");
    assert_eq!(failed.stdout_tail, "partial");
}

#[test]
fn freebsd_update_failing_for_real_still_fails() {
    let fake = Fake::default().answer(
        "PAGER=cat freebsd-update fetch --not-running-from-cron",
        exit(1, "Cannot upgrade from a version that is not a release.\n"),
    );
    let (result, ran) = run(FREEBSD, fake, Task::FullUpgrade, ROOT);
    assert!(matches!(result, Err(Error::Failed { .. })), "{result:?}");
    assert_eq!(ran.len(), 2, "install must not run after a failed fetch");
}

#[test]
fn a_command_that_cannot_start_is_an_io_error() {
    let fake = Fake::default().missing("apt-get update");
    let (result, _) = run(linux(LinuxManager::Apt), fake, Task::Refresh, ROOT);
    assert!(
        matches!(result, Err(Error::Io { ref context, .. }) if context == "apt-get update"),
        "{result:?}"
    );
}

#[test]
fn pacman_cleanup_skips_removal_without_orphans() {
    let fake = Fake::default().answer("pacman -Qdtq", exit(1, ""));
    let (outcome, ran) = run(linux(LinuxManager::Pacman), fake, Task::Cleanup, ROOT);
    assert_eq!(ran, ["pacman -Qdtq", "pacman -Sc --noconfirm"]);
    let outcome = outcome.unwrap();
    assert!(
        matches!(outcome.steps[1].state, StepState::Skipped { .. }),
        "{outcome:?}"
    );
    assert_eq!(outcome.changed(), Some(false));
}

#[test]
fn pacman_cleanup_removes_listed_orphans() {
    let fake = Fake::default()
        .answer("pacman -Qdtq", exit(0, "gnu-free-fonts\npython-wheel\n"))
        .answer(
            "pacman -Rns --noconfirm gnu-free-fonts python-wheel",
            exit(
                0,
                "checking dependencies...\n\nPackages (2) gnu-free-fonts-1  python-wheel-2\n",
            ),
        );
    let (outcome, ran) = run(linux(LinuxManager::Pacman), fake, Task::Cleanup, ROOT);
    assert_eq!(
        ran[1],
        "pacman -Rns --noconfirm gnu-free-fonts python-wheel"
    );
    let outcome = outcome.unwrap();
    assert_eq!(outcome.changes().map(|changes| changes.removed), Some(2));
}

#[test]
fn pacman_dry_run_cleanup_shows_the_pipe() {
    let options = Options {
        dry_run: true,
        privilege: Privilege::Sudo,
    };
    let (outcome, _) = run(
        linux(LinuxManager::Pacman),
        Fake::default(),
        Task::Cleanup,
        options,
    );
    let commands: Vec<_> = outcome
        .unwrap()
        .steps
        .into_iter()
        .filter_map(|step| step.command)
        .collect();
    assert_eq!(
        commands,
        [
            "pacman -Qdtq",
            "sudo pacman -Rns --noconfirm <packages listed by `pacman -Qdtq`>",
            "sudo pacman -Sc --noconfirm",
        ]
    );
}

#[test]
fn notes_are_steps_that_change_nothing() {
    let (outcome, ran) = run(
        linux(LinuxManager::Pacman),
        Fake::default(),
        Task::Refresh,
        ROOT,
    );
    assert_eq!(ran, Vec::<String>::new());
    let outcome = outcome.unwrap();
    assert!(matches!(outcome.steps[0].state, StepState::Note { .. }));
    assert_eq!(outcome.changed(), Some(false));
}

#[test]
fn apt_upgrade_counts_packages() {
    let fake = Fake::default().answer(
        "DEBIAN_FRONTEND=noninteractive apt-get -o Dpkg::Options::=--force-confdef -o Dpkg::Options::=--force-confold full-upgrade -y",
        exit(0, "3 upgraded, 1 newly installed, 0 to remove and 0 not upgraded.\n"),
    );
    let (outcome, _) = run(linux(LinuxManager::Apt), fake, Task::FullUpgrade, ROOT);
    let changes = outcome.unwrap().changes().unwrap();
    assert_eq!((changes.upgraded, changes.installed), (3, 1));
}

#[test]
fn host_reports_its_manager() {
    let host = Host::new(FREEBSD, Fake::default());
    assert_eq!(host.manager(), "pkg");
    assert_eq!(host.platform(), &FREEBSD);
}

#[test]
fn detect_finds_a_platform_or_says_unsupported() {
    match Host::detect() {
        Ok(host) => assert_ne!(host.manager(), ""),
        // Not Linux or a BSD; or a Linux host with an unreadable os-release.
        Err(Error::Unsupported(_) | Error::Io { .. }) => {}
        Err(other) => panic!("{other}"),
    }
}

fn wanted(backend: &str) -> PackageSet {
    let canonical = ["protobuf-compiler".to_string(), "pkg-config".to_string()];
    PackageSet::resolve(backend, &canonical)
}

const ROOT_APT_INSTALL: &str = "DEBIAN_FRONTEND=noninteractive apt-get -o Dpkg::Options::=--force-confdef -o Dpkg::Options::=--force-confold install -y pkg-config protobuf-compiler";

#[test]
fn install_dry_run_plans_native_names_and_runs_nothing() {
    let host = Host::new(FREEBSD, Fake::default());
    let options = Options {
        dry_run: true,
        privilege: Privilege::Sudo,
    };
    let outcome = host.install(&wanted("freebsd"), &options).unwrap();
    assert_eq!(host.runner.ran(), Vec::<String>::new());
    assert_eq!(outcome.task, Task::Install);
    assert_eq!(outcome.manager, "pkg");
    assert_eq!(outcome.steps.len(), 1);
    assert_eq!(outcome.steps[0].state, StepState::Planned);
    assert_eq!(
        outcome.steps[0].command.as_deref(),
        Some("sudo ASSUME_ALWAYS_YES=yes pkg install -y pkgconf protobuf")
    );
}

#[test]
fn install_as_root_fetches_a_missing_apt_index_first() {
    let host = Host::new(linux(LinuxManager::Apt), Fake::default());
    host.install(&wanted("apt"), &ROOT).unwrap();
    assert_eq!(host.runner.ran(), ["apt-get update", ROOT_APT_INSTALL]);
}

#[test]
fn install_skips_the_index_apt_already_has() {
    let fake = Fake::default().dir(
        "/var/lib/apt/lists",
        &[
            "lock",
            "deb.debian.org_debian_dists_trixie_main_binary-amd64_Packages.lz4",
        ],
    );
    let host = Host::new(linux(LinuxManager::Apt), fake);
    host.install(&wanted("apt"), &ROOT).unwrap();
    assert_eq!(host.runner.ran(), [ROOT_APT_INSTALL]);
}

#[test]
fn install_counts_what_it_installed() {
    let fake = Fake::default()
        .dir("/var/lib/apt/lists", &["x_Packages"])
        .answer(
            ROOT_APT_INSTALL,
            exit(
                0,
                "0 upgraded, 4 newly installed, 0 to remove and 0 not upgraded.\n",
            ),
        );
    let host = Host::new(linux(LinuxManager::Apt), fake);
    let outcome = host.install(&wanted("apt"), &ROOT).unwrap();
    assert_eq!(outcome.changed(), Some(true));
    assert_eq!(
        outcome.changes(),
        Some(Changes {
            installed: 4,
            ..Changes::default()
        })
    );
}

#[test]
fn install_failure_carries_the_outcome() {
    let fake = Fake::default().answer(
        "pacman -S --needed --noconfirm pkgconf protobuf",
        exit(1, "error: target not found: protobuf\n"),
    );
    let host = Host::new(linux(LinuxManager::Pacman), fake);
    let Err(Error::Failed { code, outcome, .. }) = host.install(&wanted("pacman"), &ROOT) else {
        panic!("a failed install must be Error::Failed");
    };
    assert_eq!(code, Some(1));
    assert_eq!(outcome.task, Task::Install);
    assert_eq!(outcome.steps[0].state, StepState::Failed);
}

#[test]
fn install_refuses_packages_resolved_for_another_backend() {
    let host = Host::new(linux(LinuxManager::Apk), Fake::default());
    let Err(Error::WrongBackend {
        host: key,
        packages,
    }) = host.install(&wanted("apt"), &ROOT)
    else {
        panic!("apt's names must not reach apk");
    };
    assert_eq!((key, packages.as_str()), ("apk", "apt"));
    assert_eq!(host.runner.ran(), Vec::<String>::new());
}

#[test]
fn install_of_nothing_runs_nothing() {
    let host = Host::new(FREEBSD, Fake::default());
    let outcome = host
        .install(&PackageSet::resolve("freebsd", &[]), &ROOT)
        .unwrap();
    assert_eq!(host.runner.ran(), Vec::<String>::new());
    assert_eq!(outcome.changed(), Some(false));
    let StepState::Note { text } = &outcome.steps[0].state else {
        panic!("an empty install is a note");
    };
    assert_eq!(text, "no packages to install");
}

#[test]
fn install_without_files_fetches_apts_index_rather_than_guess() {
    let fake = Fake::default()
        .dir("/var/lib/apt/lists", &["x_Packages"])
        .without_files();
    let host = Host::new(linux(LinuxManager::Apt), fake);
    host.install(&wanted("apt"), &ROOT).unwrap();
    assert_eq!(host.runner.ran(), ["apt-get update", ROOT_APT_INSTALL]);
}

#[test]
fn reboot_check_without_files_is_unsupported_not_a_guess() {
    let fake = Fake::default()
        .answer("uname -r", exit(0, "6.1.0-25-amd64\n"))
        .file("/var/run/reboot-required", "", 0)
        .without_files();
    let host = Host::new(linux(LinuxManager::Apt), fake);
    let error = host.reboot_required().unwrap_err();
    assert!(matches!(error, Error::Unsupported(_)), "{error}");
    assert_eq!(host.runner.ran(), Vec::<String>::new());
}
