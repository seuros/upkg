//! What a step changed, read off the manager's own summary.

use serde::{Deserialize, Serialize};

/// Package changes made by a step. Metadata refreshes and cache cleaning
/// change no packages and count as zero.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Changes {
    pub upgraded: u32,
    pub installed: u32,
    pub removed: u32,
}

impl Changes {
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.upgraded == 0 && self.installed == 0 && self.removed == 0
    }

    #[must_use]
    pub const fn plus(self, other: Self) -> Self {
        Self {
            upgraded: self.upgraded + other.upgraded,
            installed: self.installed + other.installed,
            removed: self.removed + other.removed,
        }
    }
}

/// How to read a step's output for changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Counter {
    /// Touches metadata or caches, never packages.
    Nothing,
    /// `N upgraded, N newly installed, N to remove and N not upgraded.`
    Apt,
    /// dnf4 `Upgrade  3 Packages` or dnf5 `Upgrading: 3 packages`.
    Dnf,
    /// `Packages (N) ...`; whether they were upgraded or removed is the step's.
    Pacman(Direction),
    /// `The following N packages are going to be upgraded:`.
    Zypper,
    /// `(1/3) Upgrading foo (1.0 -> 1.1)`.
    Apk,
    /// pkg(8): `Number of packages to be upgraded: N`.
    Pkg,
    /// `freebsd-update install`: base changed unless it had nothing to do.
    FreebsdUpdate,
}

/// Which way a pacman transaction went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Upgraded,
    Removed,
}

/// What a successful step changed: whether anything did, and package counts.
/// `None` where the output doesn't say. `settled` marks a step that exited
/// non-zero with a nothing-to-do message.
pub fn count(counter: Counter, stdout: &str, settled: bool) -> (Option<bool>, Option<Changes>) {
    let changes = match counter {
        Counter::Nothing => Some(Changes::default()),
        Counter::Apt => apt(stdout),
        Counter::Dnf => dnf(stdout),
        Counter::Pacman(direction) => pacman(stdout, direction),
        Counter::Zypper => zypper(stdout),
        Counter::Apk => Some(apk(stdout)),
        Counter::Pkg => pkg(stdout),
        Counter::FreebsdUpdate if settled => Some(Changes::default()),
        // Base files changed, but they are not packages.
        Counter::FreebsdUpdate => return (Some(true), None),
    };
    (changes.map(|changes| !changes.is_empty()), changes)
}

fn number(token: Option<&str>) -> Option<u32> {
    token?
        .trim_start_matches('(')
        .trim_end_matches(')')
        .parse()
        .ok()
}

fn apt(stdout: &str) -> Option<Changes> {
    stdout.lines().find_map(|line| {
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.as_slice() {
            [
                upgraded,
                "upgraded,",
                installed,
                "newly",
                "installed,",
                removed,
                "to",
                "remove",
                ..,
            ] => Some(Changes {
                upgraded: upgraded.parse().ok()?,
                installed: installed.parse().ok()?,
                removed: removed.parse().ok()?,
            }),
            _ => None,
        }
    })
}

fn dnf(stdout: &str) -> Option<Changes> {
    if stdout.lines().any(|line| line.trim() == "Nothing to do.") {
        return Some(Changes::default());
    }
    let mut changes = None::<Changes>;
    for line in stdout.lines() {
        let mut words = line.split_whitespace();
        let (Some(verb), Some(count), Some(unit)) = (words.next(), words.next(), words.next())
        else {
            continue;
        };
        let Ok(count) = count.parse::<u32>() else {
            continue;
        };
        if !unit.to_ascii_lowercase().starts_with("package") {
            continue;
        }
        let total = changes.get_or_insert_default();
        match verb.trim_end_matches(':') {
            "Upgrade" | "Upgrading" => total.upgraded += count,
            "Install" | "Installing" => total.installed += count,
            "Remove" | "Removing" => total.removed += count,
            _ => {}
        }
    }
    changes
}

fn pacman(stdout: &str, direction: Direction) -> Option<Changes> {
    if stdout.contains("there is nothing to do") {
        return Some(Changes::default());
    }
    let count = stdout.lines().find_map(|line| {
        let mut words = line.split_whitespace();
        (words.next() == Some("Packages"))
            .then(|| number(words.next()))
            .flatten()
    })?;
    Some(match direction {
        Direction::Upgraded => Changes {
            upgraded: count,
            ..Changes::default()
        },
        Direction::Removed => Changes {
            removed: count,
            ..Changes::default()
        },
    })
}

fn zypper(stdout: &str) -> Option<Changes> {
    if stdout.lines().any(|line| line.trim() == "Nothing to do.") {
        return Some(Changes::default());
    }
    let mut changes = None::<Changes>;
    for line in stdout.lines() {
        let Some(rest) = line.trim().strip_prefix("The following ") else {
            continue;
        };
        // "5 packages are ...", "2 NEW packages are ...", "NEW package is ...".
        let count = rest
            .split_whitespace()
            .next()
            .and_then(|word| word.parse::<u32>().ok())
            .unwrap_or(1);
        let total = if rest.ends_with("going to be upgraded:") {
            &mut changes.get_or_insert_default().upgraded
        } else if rest.ends_with("going to be installed:") {
            &mut changes.get_or_insert_default().installed
        } else if rest.ends_with("going to be REMOVED:") {
            &mut changes.get_or_insert_default().removed
        } else {
            continue;
        };
        *total += count;
    }
    changes
}

fn apk(stdout: &str) -> Changes {
    let mut changes = Changes::default();
    for line in stdout.lines() {
        let Some((progress, rest)) = line.split_once(") ") else {
            continue;
        };
        if !progress.starts_with('(') {
            continue;
        }
        match rest.split_whitespace().next() {
            Some("Upgrading" | "Replacing") => changes.upgraded += 1,
            Some("Installing") => changes.installed += 1,
            Some("Purging" | "Deleting") => changes.removed += 1,
            _ => {}
        }
    }
    changes
}

fn pkg(stdout: &str) -> Option<Changes> {
    let mut changes = None::<Changes>;
    for line in stdout.lines() {
        let line = line.trim();
        if matches!(line, "Your packages are up to date." | "Nothing to do.") {
            changes.get_or_insert_default();
            continue;
        }
        let Some(rest) = line.strip_prefix("Number of packages to be ") else {
            continue;
        };
        let Some((verb, count)) = rest.split_once(':') else {
            continue;
        };
        let Ok(count) = count.trim().parse::<u32>() else {
            continue;
        };
        let total = changes.get_or_insert_default();
        match verb {
            "upgraded" => total.upgraded += count,
            "installed" => total.installed += count,
            "removed" => total.removed += count,
            _ => {}
        }
    }
    changes
}

#[cfg(test)]
mod tests;
