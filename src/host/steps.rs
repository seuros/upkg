//! The native commands behind each maintenance task, per platform.

use super::changes::{Counter, Direction};
use super::{Error, LinuxManager, Platform, Task};
use crate::command::CommandSpec;

/// One thing a task does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Must exit 0.
    Run { spec: CommandSpec, counter: Counter },
    /// Exit 0, or a non-zero exit whose stdout carries one of `settled_by`:
    /// a tool that reports "nothing to do" as an error.
    Settle {
        spec: CommandSpec,
        counter: Counter,
        settled_by: &'static [&'static str],
    },
    /// Ask read-only `query` for package names (one per line); when it lists
    /// any, run `remove` with them appended. Stands in for a shell pipe.
    RemoveListed {
        query: CommandSpec,
        remove: CommandSpec,
        counter: Counter,
    },
    /// Nothing to run; say why.
    Note(&'static str),
}

/// What freebsd-update prints when there is nothing to fetch or install.
/// `install` exits 2 in that case; `fetch` exits 0, but is held to the same.
const FREEBSD_UPDATE_NOTHING: &[&str] =
    &["No updates needed", "No updates are available to install"];

/// apt and dpkg must never stop to ask: debconf gets no frontend, and changed
/// config files keep the local copy (or the default where nothing was edited).
const APT_UNATTENDED: &[&str] = &[
    "-o",
    "Dpkg::Options::=--force-confdef",
    "-o",
    "Dpkg::Options::=--force-confold",
];

fn sudo(tool: &str, args: &[&str]) -> CommandSpec {
    CommandSpec::with_packages("sudo", &[&[tool], args].concat(), &[])
}

fn plain(tool: &str, args: &[&str]) -> CommandSpec {
    CommandSpec::with_packages(tool, args, &[])
}

const fn run(spec: CommandSpec, counter: Counter) -> Step {
    Step::Run { spec, counter }
}

fn apt_get(args: &[&str]) -> CommandSpec {
    sudo("apt-get", &[APT_UNATTENDED, args].concat()).env("DEBIAN_FRONTEND", "noninteractive")
}

fn freebsd_update(command: &str) -> Step {
    let mut args = vec![command];
    if command == "fetch" {
        // fetch refuses to run without a tty unless told it's automated.
        args.push("--not-running-from-cron");
    }
    Step::Settle {
        spec: sudo("freebsd-update", &args).env("PAGER", "cat"),
        counter: if command == "install" {
            Counter::FreebsdUpdate
        } else {
            Counter::Nothing
        },
        settled_by: FREEBSD_UPDATE_NOTHING,
    }
}

/// The steps for `task` on `platform`.
pub fn plan(platform: Platform, task: Task) -> Result<Vec<Step>, Error> {
    match platform {
        Platform::Linux { manager, rolling } => linux(manager, rolling, task),
        Platform::FreeBsd { pkgbase } => bsd(
            task,
            if pkgbase {
                Base::Packaged
            } else {
                Base::FreebsdUpdate
            },
        ),
        Platform::DragonFly => bsd(task, Base::Source),
    }
}

/// [`Task::Install`] has no plan of its own: it needs the packages.
const fn needs_packages() -> Error {
    Error::Unsupported("install takes packages: use Host::install")
}

/// The steps that install `packages`, already native names, unattended.
/// `fetch_index` runs `apt-get update` first, for an apt that never fetched
/// a package list (a fresh container).
pub fn install(platform: Platform, packages: &[String], fetch_index: bool) -> Vec<Step> {
    use LinuxManager::{Apk, Apt, Dnf, Opkg, Pacman, Yum, Zypper};

    let (spec, counter) = match platform {
        Platform::Linux { manager, .. } => match manager {
            Apt => (apt_get(&["install", "-y"]), Counter::Apt),
            Dnf | Yum => (sudo(manager.name(), &["install", "-y"]), Counter::Dnf),
            // --needed: what is installed stays as is.
            Pacman => (
                sudo("pacman", &["-S", "--needed", "--noconfirm"]),
                Counter::Pacman(Direction::Installed),
            ),
            Zypper => (
                sudo("zypper", &["--non-interactive", "install"]),
                Counter::Zypper,
            ),
            Opkg => (plain("opkg", &["install"]), Counter::Unknown),
            // Alpine images ship without an index; --update-cache fetches it.
            Apk => (sudo("apk", &["add", "--update-cache"]), Counter::Apk),
        },
        // ASSUME_ALWAYS_YES also answers pkg's own bootstrap in a fresh jail.
        Platform::FreeBsd { .. } | Platform::DragonFly => (
            sudo("pkg", &["install", "-y"]).env("ASSUME_ALWAYS_YES", "yes"),
            Counter::Pkg,
        ),
    };
    let mut steps = Vec::with_capacity(2);
    if fetch_index {
        steps.push(run(sudo("apt-get", &["update"]), Counter::Nothing));
    }
    steps.push(run(spec.arguments(packages.iter().cloned()), counter));
    steps
}

fn linux(manager: LinuxManager, rolling: bool, task: Task) -> Result<Vec<Step>, Error> {
    use LinuxManager::{Apk, Apt, Dnf, Opkg, Pacman, Yum, Zypper};

    let steps = match (task, manager) {
        (Task::Refresh, Apt) => vec![run(sudo("apt-get", &["update"]), Counter::Nothing)],
        (Task::Refresh, Dnf | Yum) => {
            vec![run(sudo(manager.name(), &["makecache"]), Counter::Nothing)]
        }
        (Task::Refresh, Pacman) => vec![Step::Note(
            "pacman has no safe metadata-only refresh (-Sy alone invites partial upgrades); \
             `upgrade --full` syncs with -Syu",
        )],
        (Task::Refresh, Zypper) => vec![run(
            sudo("zypper", &["--non-interactive", "refresh"]),
            Counter::Nothing,
        )],
        (Task::Refresh, Opkg) => vec![run(plain("opkg", &["update"]), Counter::Nothing)],
        (Task::Refresh, Apk) => vec![run(sudo("apk", &["update"]), Counter::Nothing)],

        (Task::FullUpgrade, Apt) => vec![run(apt_get(&["full-upgrade", "-y"]), Counter::Apt)],
        (Task::FullUpgrade, Dnf) => vec![run(sudo("dnf", &["upgrade", "-y"]), Counter::Dnf)],
        // `upgrade` is `update` with obsoletes always processed.
        (Task::FullUpgrade, Yum) => vec![run(sudo("yum", &["upgrade", "-y"]), Counter::Dnf)],
        (Task::FullUpgrade, Pacman) => vec![run(
            sudo("pacman", &["-Syu", "--noconfirm"]),
            Counter::Pacman(Direction::Upgraded),
        )],
        // Tumbleweed is only supported through dist-upgrade; on a fixed
        // release (Leap, SLES) dist-upgrade is for moving between releases
        // and `update` is the in-release path.
        (Task::FullUpgrade, Zypper) => {
            let verb = if rolling { "dist-upgrade" } else { "update" };
            vec![run(
                sudo("zypper", &["--non-interactive", verb]),
                Counter::Zypper,
            )]
        }
        (Task::FullUpgrade, Apk) => vec![run(
            sudo("apk", &["upgrade", "--update-cache"]),
            Counter::Apk,
        )],
        (Task::FullUpgrade, Opkg) => {
            return Err(Error::Unsupported(
                "OpenWrt upgrades the whole system with sysupgrade, not opkg",
            ));
        }

        // apt never autoremoves the running kernel, nor the newest two.
        (Task::Cleanup, Apt) => vec![
            run(apt_get(&["autoremove", "--purge", "-y"]), Counter::Apt),
            run(sudo("apt-get", &["clean"]), Counter::Nothing),
        ],
        // dnf and yum protect the running kernel (protect_running_kernel).
        (Task::Cleanup, Dnf | Yum) => vec![
            run(sudo(manager.name(), &["autoremove", "-y"]), Counter::Dnf),
            run(sudo(manager.name(), &["clean", "all"]), Counter::Nothing),
        ],
        // -Qdt lists dependencies nothing needs; a kernel is installed
        // explicitly, so it is never one of them.
        (Task::Cleanup, Pacman) => vec![
            Step::RemoveListed {
                query: plain("pacman", &["-Qdtq"]),
                remove: sudo("pacman", &["-Rns", "--noconfirm"]),
                counter: Counter::Pacman(Direction::Removed),
            },
            run(sudo("pacman", &["-Sc", "--noconfirm"]), Counter::Nothing),
        ],
        (Task::Cleanup, Zypper) => vec![
            Step::Note("zypper has no autoremove; unneeded packages are left in place"),
            run(
                sudo("zypper", &["--non-interactive", "clean"]),
                Counter::Nothing,
            ),
        ],
        (Task::Cleanup, Apk) => vec![run(sudo("apk", &["cache", "clean"]), Counter::Nothing)],
        (Task::Cleanup, Opkg) => vec![Step::Note(
            "opkg keeps no package cache and tracks no orphans; nothing to clean",
        )],
        (Task::Install, _) => return Err(needs_packages()),
    };
    Ok(steps)
}

/// Who updates a BSD base system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Base {
    /// freebsd-update(8) binary updates.
    FreebsdUpdate,
    /// pkgbase: base is packages, `pkg upgrade` already covered it.
    Packaged,
    /// `DragonFly`: rebuilt from source, out of upkg's hands.
    Source,
}

fn pkg(args: &[&str], counter: Counter) -> Step {
    run(sudo("pkg", args), counter)
}

fn bsd(task: Task, base: Base) -> Result<Vec<Step>, Error> {
    let steps = match task {
        Task::Refresh => vec![pkg(&["update"], Counter::Nothing)],
        Task::FullUpgrade => {
            let mut steps = vec![pkg(&["upgrade", "-y"], Counter::Pkg)];
            match base {
                Base::FreebsdUpdate => {
                    steps.extend([freebsd_update("fetch"), freebsd_update("install")]);
                }
                Base::Packaged => {}
                Base::Source => steps.push(Step::Note(
                    "DragonFly's base system is rebuilt from source (make world); upkg leaves it alone",
                )),
            }
            steps
        }
        // The running kernel lives in /boot/kernel, outside any package
        // autoremove would consider (on pkgbase it is installed explicitly).
        Task::Cleanup => vec![
            pkg(&["autoremove", "-y"], Counter::Pkg),
            pkg(&["clean", "-ay"], Counter::Nothing),
        ],
        Task::Install => return Err(needs_packages()),
    };
    Ok(steps)
}

#[cfg(test)]
mod tests;
