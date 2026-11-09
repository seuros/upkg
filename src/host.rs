//! Host maintenance: keep a machine's packages, kernel and base system current.
//!
//! ```no_run
//! use upkg::host::{Host, Options};
//!
//! # fn main() -> Result<(), upkg::host::Error> {
//! let host = Host::detect()?;
//! let options = Options::default();
//! host.refresh(&options)?;
//! let upgrade = host.full_upgrade(&options)?;
//! if upgrade.changed() != Some(false) {
//!     host.cleanup(&options)?;
//! }
//! if host.reboot_required()?.required {
//!     // schedule a reboot
//! }
//! # Ok(())
//! # }
//! ```

mod changes;
mod error;
mod outcome;
mod platform;
mod reboot;
mod runner;
mod steps;

pub use changes::Changes;
pub use error::Error;
pub use outcome::{Outcome, StepOutcome, StepState, TAIL_LINES, Task};
pub use platform::{LinuxManager, Platform};
pub use reboot::RebootStatus;
pub use runner::{Execution, Filesystem, LocalFilesystem, Runner, SystemRunner};

pub use crate::command::{CommandSpec, Privilege};
use crate::manifest::PackageSet;

use changes::count;
use outcome::tail;
use steps::Step;

/// How a task runs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Options {
    /// Plan the commands without running any of them.
    pub dry_run: bool,
    /// How privileged commands get root. [`Privilege::AlreadyRoot`] (or
    /// [`Privilege::Auto`] in a root process) adds no prefix at all.
    pub privilege: Privilege,
}

/// A machine to maintain: its platform, and the runner that touches it.
#[derive(Debug, Clone)]
pub struct Host<R: Runner = SystemRunner> {
    platform: Platform,
    runner: R,
}

impl Host {
    /// The running system, driven by real processes whose output is captured.
    ///
    /// # Errors
    ///
    /// The platform is not one upkg can maintain.
    pub fn detect() -> Result<Self, Error> {
        Self::detect_with(SystemRunner::new())
    }
}

impl<R: Runner> Host<R> {
    /// The running system, driven through `runner`.
    ///
    /// # Errors
    ///
    /// The platform is not one upkg can maintain.
    pub fn detect_with(runner: R) -> Result<Self, Error> {
        Ok(Self {
            platform: Platform::detect(&runner)?,
            runner,
        })
    }

    /// A host of a known platform; nothing is probed until a task runs.
    pub const fn new(platform: Platform, runner: R) -> Self {
        Self { platform, runner }
    }

    pub const fn platform(&self) -> &Platform {
        &self.platform
    }

    /// The package manager's name (`apt`, `dnf`, `pkg`, ...).
    pub const fn manager(&self) -> &'static str {
        self.platform.manager()
    }

    /// Refreshes package metadata only.
    ///
    /// # Errors
    ///
    /// A command failed ([`Error::Failed`] carries the outcome so far) or
    /// could not be started.
    pub fn refresh(&self, options: &Options) -> Result<Outcome, Error> {
        self.perform(Task::Refresh, options)
    }

    /// Upgrades the whole system, new kernels and the base system included.
    ///
    /// # Errors
    ///
    /// As [`Host::refresh`], or [`Error::Unsupported`] on `OpenWrt`.
    pub fn full_upgrade(&self, options: &Options) -> Result<Outcome, Error> {
        self.perform(Task::FullUpgrade, options)
    }

    /// Removes orphaned packages and clears the package cache. Never removes
    /// the running kernel.
    ///
    /// # Errors
    ///
    /// As [`Host::refresh`].
    pub fn cleanup(&self, options: &Options) -> Result<Outcome, Error> {
        self.perform(Task::Cleanup, options)
    }

    /// Whether the host must reboot to run what is installed.
    ///
    /// # Errors
    ///
    /// The check itself failed: `uname` could not run, or a probe answered
    /// something it should not.
    pub fn reboot_required(&self) -> Result<RebootStatus, Error> {
        reboot::check(self.platform, &self.runner)
    }

    /// Installs `packages`, unattended: apt fetches its index first when it
    /// never has (a fresh container), and pkg bootstraps itself if it must
    /// (a fresh jail). Resolve the set for this host with
    /// `manifest.resolve(host.platform().catalog_key())`; an empty set runs
    /// nothing.
    ///
    /// # Errors
    ///
    /// [`Error::WrongBackend`] when `packages` were resolved for another
    /// backend, or as [`Host::refresh`].
    pub fn install(&self, packages: &PackageSet, options: &Options) -> Result<Outcome, Error> {
        let host = self.platform.catalog_key();
        if packages.backend() != host {
            return Err(Error::WrongBackend {
                host,
                packages: packages.backend().to_string(),
            });
        }
        let steps = if packages.is_empty() {
            vec![Step::Note("no packages to install")]
        } else {
            steps::install(self.platform, packages.packages(), self.needs_index())
        };
        self.run_steps(Task::Install, steps, *options)
    }

    /// Runs `task`, stopping at the first failed step.
    ///
    /// # Errors
    ///
    /// As [`Host::refresh`]; [`Error::Unsupported`] for [`Task::Install`],
    /// which takes packages ([`Host::install`]).
    pub fn perform(&self, task: Task, options: &Options) -> Result<Outcome, Error> {
        self.run_steps(task, steps::plan(self.platform, task)?, *options)
    }

    fn run_steps(&self, task: Task, steps: Vec<Step>, options: Options) -> Result<Outcome, Error> {
        let privilege = self.privilege(options.privilege);
        let mut outcome = Outcome {
            task,
            manager: self.manager().to_string(),
            dry_run: options.dry_run,
            steps: Vec::new(),
        };
        for step in steps {
            self.step(step, privilege, options.dry_run, &mut outcome)?;
        }
        Ok(outcome)
    }

    /// Whether apt has never fetched a package list: apt names them
    /// `<repo>_Packages`, plus `.lz4`/`.gz` when compressed. A runner that
    /// lends no files can't say, so the index is fetched.
    fn needs_index(&self) -> bool {
        matches!(
            self.platform,
            Platform::Linux {
                manager: LinuxManager::Apt,
                ..
            }
        ) && !self.runner.filesystem().is_some_and(|files| {
            files
                .read_dir(std::path::Path::new(APT_LISTS))
                .is_ok_and(|names| names.iter().any(|name| name.contains("_Packages")))
        })
    }

    fn privilege(&self, requested: Privilege) -> Privilege {
        match requested {
            Privilege::Auto if self.runner.is_root() => Privilege::AlreadyRoot,
            Privilege::Auto
                if !self.runner.has_command("sudo") && self.runner.has_command("doas") =>
            {
                Privilege::Doas
            }
            Privilege::Auto => Privilege::Sudo,
            explicit => explicit,
        }
    }

    fn step(
        &self,
        step: Step,
        privilege: Privilege,
        dry_run: bool,
        outcome: &mut Outcome,
    ) -> Result<(), Error> {
        match step {
            Step::Note(text) => outcome.steps.push(StepOutcome::note(text)),
            Step::Run { spec, counter } => {
                self.command(&spec.escalate(privilege), counter, &[], dry_run, outcome)?;
            }
            Step::Settle {
                spec,
                counter,
                settled_by,
            } => self.command(
                &spec.escalate(privilege),
                counter,
                settled_by,
                dry_run,
                outcome,
            )?,
            Step::RemoveListed {
                query,
                remove,
                counter,
            } => {
                self.remove_listed(
                    &query,
                    remove.escalate(privilege),
                    counter,
                    dry_run,
                    outcome,
                )?;
            }
        }
        Ok(())
    }

    /// Runs `spec` and records it; a failure ends the task.
    fn command(
        &self,
        spec: &CommandSpec,
        counter: changes::Counter,
        settled_by: &[&str],
        dry_run: bool,
        outcome: &mut Outcome,
    ) -> Result<(), Error> {
        let command = spec.render();
        if dry_run {
            outcome.steps.push(StepOutcome::planned(command));
            return Ok(());
        }
        let execution = self.execute(spec, &command)?;
        let settled = !execution.success()
            && settled_by
                .iter()
                .any(|phrase| execution.stdout.contains(phrase));
        let state = if execution.success() {
            StepState::Succeeded
        } else if settled {
            StepState::Settled
        } else {
            StepState::Failed
        };
        let tally = if state == StepState::Failed {
            (None, None)
        } else {
            count(counter, &execution.stdout, settled)
        };
        record(outcome, command, state, &execution, tally)
    }

    /// `query | xargs remove`, without a shell: run `remove` only when
    /// `query` lists something. pacman exits 1 with no output for "none".
    fn remove_listed(
        &self,
        query: &CommandSpec,
        remove: CommandSpec,
        counter: changes::Counter,
        dry_run: bool,
        outcome: &mut Outcome,
    ) -> Result<(), Error> {
        let asked = query.render();
        if dry_run {
            outcome.steps.push(StepOutcome::planned(asked.clone()));
            outcome.steps.push(StepOutcome::planned(format!(
                "{} <packages listed by `{asked}`>",
                remove.render()
            )));
            return Ok(());
        }
        let execution = self.execute(query, &asked)?;
        let listed: Vec<String> = execution
            .stdout
            .split_whitespace()
            .map(ToString::to_string)
            .collect();
        let state = if execution.success() || listed.is_empty() {
            StepState::Succeeded
        } else {
            StepState::Failed
        };
        let unchanged = (Some(false), Some(Changes::default()));
        record(outcome, asked.clone(), state, &execution, unchanged)?;
        if listed.is_empty() {
            outcome.steps.push(StepOutcome::skipped(
                remove.render(),
                format!("`{asked}` listed nothing to remove"),
            ));
            return Ok(());
        }
        self.command(&remove.arguments(listed), counter, &[], false, outcome)
    }

    fn execute(&self, spec: &CommandSpec, command: &str) -> Result<Execution, Error> {
        self.runner.run(spec).map_err(|source| Error::Io {
            context: command.to_string(),
            source,
        })
    }
}

/// Where apt keeps downloaded package lists; container images ship it empty.
const APT_LISTS: &str = "/var/lib/apt/lists";

/// Appends a step that ran; turns a failed one into [`Error::Failed`].
fn record(
    outcome: &mut Outcome,
    command: String,
    state: StepState,
    execution: &Execution,
    tally: (Option<bool>, Option<Changes>),
) -> Result<(), Error> {
    let failed = state == StepState::Failed;
    outcome.steps.push(StepOutcome {
        command: Some(command.clone()),
        state,
        code: execution.code,
        stdout_tail: tail(&execution.stdout),
        stderr_tail: tail(&execution.stderr),
        changed: tally.0,
        changes: tally.1,
    });
    if failed {
        return Err(Error::Failed {
            command,
            code: execution.code,
            outcome: Box::new(outcome.clone()),
        });
    }
    Ok(())
}

#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;
