//! Does the host need a reboot to run what is installed?
//!
//! Every probe goes through the [`Runner`]; the verdicts are pure functions.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::{Error, Filesystem, LinuxManager, Platform, Runner};
use crate::command::CommandSpec;

/// The answer to "does this host need a reboot?".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebootStatus {
    pub required: bool,
    /// Why a reboot is required, or why the check can't tell.
    pub reason: Option<String>,
    /// Packages that asked for the reboot, where the system records them.
    pub packages: Vec<String>,
    /// `uname -r`.
    pub running_kernel: String,
    /// The kernel the next boot runs, where it can be determined.
    pub installed_kernel: Option<String>,
    /// RFC 3339 UTC time the reboot became necessary, where known.
    pub since: Option<String>,
}

impl RebootStatus {
    /// One line for humans.
    #[must_use]
    pub fn summary(&self) -> String {
        let mut line = match (self.required, &self.reason) {
            (true, Some(reason)) => format!("reboot required: {reason}"),
            (true, None) => "reboot required".to_string(),
            (false, Some(reason)) => format!("no reboot required ({reason})"),
            (false, None) => format!("no reboot required (running {})", self.running_kernel),
        };
        if !self.packages.is_empty() {
            line = format!("{line}; packages: {}", self.packages.join(", "));
        }
        if let Some(since) = &self.since {
            line = format!("{line}; since {since}");
        }
        line
    }

    const fn not_required(running_kernel: String) -> Self {
        Self {
            required: false,
            reason: None,
            packages: Vec::new(),
            running_kernel,
            installed_kernel: None,
            since: None,
        }
    }
}

/// Debian and Ubuntu: written by package hooks that want a reboot.
pub const DEBIAN_FLAG: &str = "/var/run/reboot-required";
const DEBIAN_PACKAGES: &str = "/var/run/reboot-required.pkgs";
/// Where Linux kernels keep their modules; merged-/usr systems first.
const MODULE_DIRS: [&str; 2] = ["/usr/lib/modules", "/lib/modules"];
const FREEBSD_KERNEL: &str = "/boot/kernel/kernel";

pub fn check(platform: Platform, runner: &impl Runner) -> Result<RebootStatus, Error> {
    let files = runner.filesystem().ok_or(Error::Unsupported(
        "the reboot check reads the host's files, and this runner lends none",
    ))?;
    let running = probe(runner, "uname", &["-r"])?;
    match platform {
        Platform::Linux { manager, .. } => Ok(linux(manager, running, runner, files)),
        Platform::FreeBsd { pkgbase } => freebsd(running, pkgbase, runner, files),
        Platform::DragonFly => Ok(RebootStatus {
            reason: Some("cannot tell: DragonFly records no installed kernel version".into()),
            ..RebootStatus::not_required(running)
        }),
    }
}

/// The first line of a probe's stdout; anything but exit 0 fails the check.
fn probe(runner: &impl Runner, program: &str, args: &[&str]) -> Result<String, Error> {
    let spec = CommandSpec::with_packages(program, args, &[]);
    let execution = runner.run(&spec).map_err(|source| Error::Io {
        context: spec.render(),
        source,
    })?;
    let line = execution.stdout.lines().next().unwrap_or_default().trim();
    if !execution.success() || line.is_empty() {
        return Err(Error::Probe(format!(
            "`{}` answered {:?} (exit {:?})",
            spec.render(),
            execution.stderr.trim(),
            execution.code
        )));
    }
    Ok(line.to_string())
}

fn since(files: &dyn Filesystem, path: &Path) -> Option<String> {
    files.modified(path).ok().map(rfc3339)
}

fn linux(
    manager: LinuxManager,
    running: String,
    runner: &impl Runner,
    files: &dyn Filesystem,
) -> RebootStatus {
    let kernels = Kernels::read(files);
    match manager {
        LinuxManager::Apt => debian(running, kernels.as_ref(), files),
        LinuxManager::Dnf | LinuxManager::Yum => match needs_restarting(manager, runner) {
            Some(Restart {
                required: true,
                packages,
            }) => RebootStatus {
                required: true,
                reason: Some(
                    "needs-restarting -r: core libraries or services were updated since boot"
                        .into(),
                ),
                packages,
                installed_kernel: kernels
                    .as_ref()
                    .and_then(|kernels| kernels.newest(&running)),
                ..RebootStatus::not_required(running)
            },
            Some(Restart {
                required: false, ..
            }) => RebootStatus {
                installed_kernel: kernels
                    .as_ref()
                    .and_then(|kernels| kernels.newest(&running)),
                ..RebootStatus::not_required(running)
            },
            None => kernel_verdict(running, kernels.as_ref(), Rule::MissingOrNewer, files),
        },
        // One kernel per package, replaced in place: only a missing module
        // tree means the running kernel is gone.
        LinuxManager::Pacman => kernel_verdict(running, kernels.as_ref(), Rule::Missing, files),
        LinuxManager::Zypper | LinuxManager::Apk | LinuxManager::Opkg => {
            kernel_verdict(running, kernels.as_ref(), Rule::MissingOrNewer, files)
        }
    }
}

fn debian(running: String, kernels: Option<&Kernels>, files: &dyn Filesystem) -> RebootStatus {
    let installed_kernel = kernels
        .as_ref()
        .and_then(|kernels| kernels.newest(&running));
    let flag = Path::new(DEBIAN_FLAG);
    if !files.exists(flag) {
        return RebootStatus {
            installed_kernel,
            ..RebootStatus::not_required(running)
        };
    }
    let packages = files
        .read_to_string(Path::new(DEBIAN_PACKAGES))
        .map(|text| package_list(&text))
        .unwrap_or_default();
    RebootStatus {
        required: true,
        reason: Some(format!("{DEBIAN_FLAG} exists")),
        packages,
        installed_kernel,
        since: since(files, flag),
        ..RebootStatus::not_required(running)
    }
}

/// `reboot-required.pkgs`: one package per line, repeats dropped.
pub fn package_list(text: &str) -> Vec<String> {
    let mut packages: Vec<String> = Vec::new();
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if !packages.iter().any(|known| known == line) {
            packages.push(line.to_string());
        }
    }
    packages
}

pub struct Restart {
    pub(crate) required: bool,
    pub(crate) packages: Vec<String>,
}

/// `needs-restarting -r`, from dnf-utils or as a dnf command. Exit 1 also
/// means "unknown command" or "no cache", so a reboot counts only when the
/// output says so; anything unclear falls back to comparing kernels.
fn needs_restarting(manager: LinuxManager, runner: &impl Runner) -> Option<Restart> {
    let spec = if runner.has_command("needs-restarting") {
        CommandSpec::with_packages("needs-restarting", &["-r"], &[])
    } else if manager == LinuxManager::Dnf {
        CommandSpec::with_packages("dnf", &["needs-restarting", "-r"], &[])
    } else {
        return None;
    };
    let execution = runner.run(&spec.env("LC_ALL", "C")).ok()?;
    restart_verdict(execution.code, &execution.stdout)
}

pub fn restart_verdict(code: Option<i32>, stdout: &str) -> Option<Restart> {
    match code {
        Some(0) => Some(Restart {
            required: false,
            packages: Vec::new(),
        }),
        Some(1) if stdout.contains("Reboot is required") => Some(Restart {
            required: true,
            packages: stdout
                .lines()
                .filter_map(|line| line.trim().strip_prefix("* "))
                .map(|name| name.trim().to_string())
                .collect(),
        }),
        _ => None,
    }
}

/// The kernels with module trees on disk.
struct Kernels {
    dir: PathBuf,
    releases: Vec<String>,
}

impl Kernels {
    fn read(files: &dyn Filesystem) -> Option<Self> {
        MODULE_DIRS.iter().find_map(|dir| {
            let dir = PathBuf::from(dir);
            let releases: Vec<String> = files
                .read_dir(&dir)
                .ok()?
                .into_iter()
                .filter(|name| name.starts_with(|c: char| c.is_ascii_digit()))
                .collect();
            (!releases.is_empty()).then_some(Self { dir, releases })
        })
    }

    fn newest(&self, running: &str) -> Option<String> {
        newest_kernel(&self.releases, running)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    /// Required when the running kernel's modules are gone.
    Missing,
    /// That, or a newer kernel of the same flavor is installed.
    MissingOrNewer,
}

fn kernel_verdict(
    running: String,
    kernels: Option<&Kernels>,
    rule: Rule,
    files: &dyn Filesystem,
) -> RebootStatus {
    let Some(kernels) = kernels else {
        return RebootStatus {
            reason: Some(format!(
                "cannot tell: no kernel modules under {}",
                MODULE_DIRS.join(" or ")
            )),
            ..RebootStatus::not_required(running)
        };
    };
    let newest = kernels.newest(&running);
    let verdict = verdict(&running, &kernels.releases, newest.as_deref(), rule);
    let since = verdict
        .and(newest.as_ref())
        .and_then(|newest| since(files, &kernels.dir.join(newest)));
    RebootStatus {
        required: verdict.is_some(),
        reason: verdict.map(|verdict| verdict.reason(&running, newest.as_deref(), &kernels.dir)),
        installed_kernel: newest,
        since,
        ..RebootStatus::not_required(running)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Missing,
    Newer,
}

impl Verdict {
    fn reason(self, running: &str, newest: Option<&str>, dir: &Path) -> String {
        match self {
            Self::Missing => format!(
                "running kernel {running} has no modules under {}; it was upgraded or removed",
                dir.display()
            ),
            Self::Newer => format!(
                "kernel {} is installed but {running} is running",
                newest.unwrap_or("?")
            ),
        }
    }
}

/// Why a reboot is required, if it is.
pub fn verdict(
    running: &str,
    releases: &[String],
    newest: Option<&str>,
    rule: Rule,
) -> Option<Verdict> {
    if !releases.iter().any(|release| release == running) {
        return Some(Verdict::Missing);
    }
    match (rule, newest) {
        (Rule::MissingOrNewer, Some(newest))
            if version_cmp(newest, running) == Ordering::Greater =>
        {
            Some(Verdict::Newer)
        }
        _ => None,
    }
}

/// A kernel flavor: a last segment that starts with a letter (`lts`, `virt`,
/// `amd64`, `default`), or empty for releases without one (Fedora, Arch).
pub fn flavor(release: &str) -> &str {
    release
        .rsplit_once('-')
        .map(|(_, last)| last)
        .filter(|last| {
            last.starts_with(|c: char| c.is_ascii_alphabetic())
                && last.chars().all(|c| c.is_ascii_alphanumeric())
        })
        .unwrap_or_default()
}

/// The newest of `releases` with the running kernel's flavor.
pub fn newest_kernel(releases: &[String], running: &str) -> Option<String> {
    releases
        .iter()
        .filter(|release| flavor(release) == flavor(running))
        .max_by(|a, b| version_cmp(a, b))
        .cloned()
}

/// Natural order: digit runs compare as numbers, everything else as text.
pub fn version_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a, b);
    loop {
        match (a.is_empty(), b.is_empty()) {
            (true, true) => return Ordering::Equal,
            (true, false) => return Ordering::Less,
            (false, true) => return Ordering::Greater,
            (false, false) => {}
        }
        let (chunk_a, rest_a) = split_chunk(a);
        let (chunk_b, rest_b) = split_chunk(b);
        let order = match (chunk_a.parse::<u64>(), chunk_b.parse::<u64>()) {
            (Ok(x), Ok(y)) => x.cmp(&y),
            _ => chunk_a.cmp(chunk_b),
        };
        if order != Ordering::Equal {
            return order;
        }
        (a, b) = (rest_a, rest_b);
    }
}

/// The leading run of digits, or of non-digits.
fn split_chunk(text: &str) -> (&str, &str) {
    let digits = text.starts_with(|c: char| c.is_ascii_digit());
    let end = text
        .find(|c: char| c.is_ascii_digit() != digits)
        .unwrap_or(text.len());
    text.split_at(end)
}

fn freebsd(
    running: String,
    pkgbase: bool,
    runner: &impl Runner,
    files: &dyn Filesystem,
) -> Result<RebootStatus, Error> {
    let installed = probe(runner, "freebsd-version", &["-k"])?;
    let pending = !pkgbase && updates_ready(runner);
    let mut status = freebsd_verdict(running, installed, pending);
    if status.required {
        status.since = since(files, Path::new(FREEBSD_KERNEL));
    }
    Ok(status)
}

/// `freebsd-update updatesready`: 0 when fetched updates wait to be
/// installed, 2 when none do. Needs root to read its workdir; anything else
/// is no answer.
fn updates_ready(runner: &impl Runner) -> bool {
    let spec = CommandSpec::with_packages("freebsd-update", &["updatesready"], &[]);
    runner.run(&spec).is_ok_and(|execution| execution.success())
}

/// FreeBSD needs a reboot when the kernel on disk is not the one running.
pub fn freebsd_verdict(running: String, installed: String, pending: bool) -> RebootStatus {
    let required = installed != running;
    let reason = if required {
        Some(format!(
            "kernel {installed} is installed but {running} is running"
        ))
    } else if pending {
        Some("freebsd-update fetched updates that `freebsd-update install` has not applied".into())
    } else {
        None
    };
    RebootStatus {
        required,
        reason,
        installed_kernel: Some(installed),
        ..RebootStatus::not_required(running)
    }
}

/// `time` as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn rfc3339(time: SystemTime) -> String {
    let seconds = time
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let (days, of_day) = (seconds / 86_400, seconds % 86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        of_day / 3600,
        of_day % 3600 / 60,
        of_day % 60
    )
}

/// Days since 1970-01-01 to a proleptic Gregorian date (Howard Hinnant's
/// `civil_from_days`, for the non-negative days a file time can have).
const fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let day_of_era = z % 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + if month <= 2 { 1 } else { 0 };
    (year, month, day)
}

#[cfg(test)]
mod tests;
