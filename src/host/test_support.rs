//! A scripted [`Runner`] for host tests.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::{CommandSpec, Execution, Filesystem, LinuxManager, Platform, Runner};

#[derive(Default)]
pub(super) struct Fake {
    /// Answers by rendered command; anything unscripted exits 0 silently.
    answers: HashMap<String, Execution>,
    /// Commands that cannot start at all.
    missing: Vec<String>,
    /// Lends no files, like a runner driving another machine.
    no_files: bool,
    files: HashMap<PathBuf, (String, SystemTime)>,
    dirs: HashMap<PathBuf, Vec<String>>,
    commands: Vec<String>,
    root: bool,
    pub(super) ran: RefCell<Vec<String>>,
}

pub(super) const fn linux(manager: LinuxManager) -> Platform {
    Platform::Linux {
        manager,
        rolling: false,
    }
}

pub(super) fn exit(code: i32, stdout: &str) -> Execution {
    Execution {
        code: Some(code),
        stdout: stdout.to_string(),
        stderr: String::new(),
    }
}

impl Fake {
    pub(super) fn answer(mut self, command: &str, execution: Execution) -> Self {
        self.answers.insert(command.to_string(), execution);
        self
    }

    pub(super) fn missing(mut self, command: &str) -> Self {
        self.missing.push(command.to_string());
        self
    }

    /// A file modified `at` seconds after the epoch.
    pub(super) fn file(mut self, path: &str, text: &str, at: u64) -> Self {
        self.files.insert(
            PathBuf::from(path),
            (text.to_string(), UNIX_EPOCH + Duration::from_secs(at)),
        );
        self
    }

    pub(super) fn dir(mut self, path: &str, entries: &[&str]) -> Self {
        self.dirs.insert(
            PathBuf::from(path),
            entries.iter().map(ToString::to_string).collect(),
        );
        self
    }

    pub(super) fn command(mut self, name: &str) -> Self {
        self.commands.push(name.to_string());
        self
    }

    pub(super) const fn without_files(mut self) -> Self {
        self.no_files = true;
        self
    }

    pub(super) const fn root(mut self) -> Self {
        self.root = true;
        self
    }

    pub(super) fn ran(&self) -> Vec<String> {
        self.ran.borrow().clone()
    }
}

impl Runner for Fake {
    fn run(&self, spec: &CommandSpec) -> io::Result<Execution> {
        let command = spec.render();
        self.ran.borrow_mut().push(command.clone());
        if self.missing.contains(&command) {
            return Err(io::Error::new(io::ErrorKind::NotFound, "not found"));
        }
        Ok(self
            .answers
            .get(&command)
            .cloned()
            .unwrap_or_else(|| exit(0, "")))
    }

    fn is_root(&self) -> bool {
        self.root
    }

    fn has_command(&self, name: &str) -> bool {
        self.commands.iter().any(|known| known == name) || (name == "sudo" && !self.root)
    }

    fn filesystem(&self) -> Option<&dyn Filesystem> {
        (!self.no_files).then_some(self as &dyn Filesystem)
    }
}

impl Filesystem for Fake {
    fn exists(&self, path: &Path) -> bool {
        self.files.contains_key(path) || self.dirs.contains_key(path)
    }

    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        self.files
            .get(path)
            .map(|(text, _)| text.clone())
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }

    fn modified(&self, path: &Path) -> io::Result<SystemTime> {
        self.files
            .get(path)
            .map(|(_, at)| *at)
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }

    fn read_dir(&self, dir: &Path) -> io::Result<Vec<String>> {
        self.dirs
            .get(dir)
            .cloned()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }
}
