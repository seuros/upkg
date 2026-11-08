//! What a maintenance task did, step by step.

use serde::{Deserialize, Serialize};

use super::Changes;

/// A host-maintenance task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Task {
    /// Refresh package metadata only.
    Refresh,
    /// Upgrade everything, kernels and base system included.
    FullUpgrade,
    /// Remove orphaned packages and clear the package cache.
    Cleanup,
}

impl Task {
    /// The `upkg` command for this task.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Refresh => "refresh",
            Self::FullUpgrade => "upgrade --full",
            Self::Cleanup => "cleanup",
        }
    }
}

/// How a step ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum StepState {
    /// Dry run: shown, not run.
    Planned,
    /// Exited 0.
    Succeeded,
    /// Exited non-zero while saying there was nothing to do.
    Settled,
    /// Not run, because an earlier probe made it pointless.
    Skipped { reason: String },
    /// Nothing to run on this platform.
    Note { text: String },
    /// Exited non-zero; the task stopped here.
    Failed,
}

/// One command of a task, and how it went.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepOutcome {
    /// The command as a shell would show it; `None` for a note.
    pub command: Option<String>,
    #[serde(flatten)]
    pub state: StepState,
    /// The exit code, when the step ran to an exit.
    pub code: Option<i32>,
    /// The last lines of stdout and stderr.
    pub stdout_tail: String,
    pub stderr_tail: String,
    /// Whether packages or the base system changed; `None` when the output
    /// doesn't say, or the step didn't run.
    pub changed: Option<bool>,
    /// Package counts, when the manager's summary could be read.
    pub changes: Option<Changes>,
}

impl StepOutcome {
    pub(crate) const fn planned(command: String) -> Self {
        Self::new(Some(command), StepState::Planned)
    }

    pub(crate) fn note(text: &str) -> Self {
        Self {
            changed: Some(false),
            changes: Some(Changes::default()),
            ..Self::new(
                None,
                StepState::Note {
                    text: text.to_string(),
                },
            )
        }
    }

    pub(crate) fn skipped(command: String, reason: String) -> Self {
        Self {
            changed: Some(false),
            changes: Some(Changes::default()),
            ..Self::new(Some(command), StepState::Skipped { reason })
        }
    }

    pub(crate) const fn new(command: Option<String>, state: StepState) -> Self {
        Self {
            command,
            state,
            code: None,
            stdout_tail: String::new(),
            stderr_tail: String::new(),
            changed: None,
            changes: None,
        }
    }
}

/// Everything a task did, in order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outcome {
    pub task: Task,
    /// The package manager that did it (`apt`, `pkg`, ...).
    pub manager: String,
    pub dry_run: bool,
    pub steps: Vec<StepOutcome>,
}

impl Outcome {
    /// Whether the task changed packages or the base system: `Some(true)`
    /// when any step did, `None` when some step can't say, else `Some(false)`.
    #[must_use]
    pub fn changed(&self) -> Option<bool> {
        let mut known = true;
        for step in &self.steps {
            match step.changed {
                Some(true) => return Some(true),
                Some(false) => {}
                None => known = false,
            }
        }
        known.then_some(false)
    }

    /// Package counts summed over every step, when every step reported them.
    #[must_use]
    pub fn changes(&self) -> Option<Changes> {
        self.steps
            .iter()
            .try_fold(Changes::default(), |total, step| {
                Some(total.plus(step.changes?))
            })
    }
}

/// How many trailing lines of each stream a step keeps.
pub const TAIL_LINES: usize = 40;

/// The last [`TAIL_LINES`] lines of `text`.
pub fn tail(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(TAIL_LINES)..].join("\n")
}

#[cfg(test)]
mod tests;
