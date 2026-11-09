//! The seam between upkg and the machine: commands, and the few files some
//! answers read.

use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::Stdio;
use std::time::SystemTime;

use crate::command::{self, CommandSpec};

/// How a command ended, and what it printed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Execution {
    /// The exit code; `None` when a signal ended the process.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Execution {
    #[must_use]
    pub const fn success(&self) -> bool {
        matches!(self.code, Some(0))
    }
}

/// Runs commands on the machine a [`Host`](super::Host) maintains.
///
/// [`Runner::run`] is all a runner must do. Everything else it can be asked
/// defaults to "don't know", never to the machine this process runs on: a
/// runner that drives a container, a jail or another machine is safe as
/// written, and says more only where it knows more. [`SystemRunner`] is
/// the one that answers for the running system.
pub trait Runner {
    /// Runs `spec` to completion and returns its exit status and output.
    ///
    /// # Errors
    ///
    /// The process could not be started or its output could not be read.
    fn run(&self, spec: &CommandSpec) -> io::Result<Execution>;

    /// Whether commands run as root. Decides what
    /// [`Privilege::Auto`](crate::command::Privilege::Auto) means; when
    /// unknown (`false`), privileged commands go through `sudo`.
    fn is_root(&self) -> bool {
        false
    }

    /// Whether `name` is an executable where commands run. Unknown
    /// (`false`) by default.
    fn has_command(&self, _name: &str) -> bool {
        false
    }

    /// The files of the machine commands run on, for the few answers that
    /// read them: Linux detection (`/etc/os-release`), the reboot check, and
    /// whether apt ever fetched an index. `None` by default, and those
    /// answers then fail with [`Error::Unsupported`](super::Error::Unsupported)
    /// (an install fetches apt's index rather than guess) instead of
    /// reading this process's own files.
    fn filesystem(&self) -> Option<&dyn Filesystem> {
        None
    }
}

/// Read-only access to a machine's files, as a [`Runner`] lends it.
pub trait Filesystem {
    fn exists(&self, path: &Path) -> bool;

    /// # Errors
    ///
    /// As [`fs::read_to_string`].
    fn read_to_string(&self, path: &Path) -> io::Result<String>;

    /// When `path` was last modified.
    ///
    /// # Errors
    ///
    /// As [`fs::metadata`].
    fn modified(&self, path: &Path) -> io::Result<SystemTime>;

    /// The names of the entries in `dir`.
    ///
    /// # Errors
    ///
    /// As [`fs::read_dir`].
    fn read_dir(&self, dir: &Path) -> io::Result<Vec<String>>;
}

/// The running system's own files.
#[derive(Debug, Clone, Copy, Default)]
pub struct LocalFilesystem;

impl Filesystem for LocalFilesystem {
    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        fs::read_to_string(path)
    }

    fn modified(&self, path: &Path) -> io::Result<SystemTime> {
        fs::metadata(path)?.modified()
    }

    fn read_dir(&self, dir: &Path) -> io::Result<Vec<String>> {
        fs::read_dir(dir)?
            .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
            .collect()
    }
}

/// Runs real processes on the running system, and answers for it.
///
/// Its effective user, its `PATH` and its files ([`LocalFilesystem`]) are
/// this machine's. stdin is closed so nothing can stop to ask a question;
/// stdout and stderr are captured, and echoed live when asked.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemRunner {
    echo: bool,
}

impl SystemRunner {
    /// Captures output without printing it.
    #[must_use]
    pub const fn new() -> Self {
        Self { echo: false }
    }

    /// Captures output and passes it through to this process's stdout and
    /// stderr as it arrives.
    #[must_use]
    pub const fn echoing() -> Self {
        Self { echo: true }
    }
}

impl Runner for SystemRunner {
    fn run(&self, spec: &CommandSpec) -> io::Result<Execution> {
        let mut child = spec
            .clone()
            .into_command()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let echo = self.echo;
        let (stdout, stderr) = std::thread::scope(|scope| {
            let out = scope.spawn(move || drain(stdout, echo.then(io::stdout)));
            let err = drain(stderr, echo.then(io::stderr));
            (out.join().unwrap_or_else(|_| Ok(Vec::new())), err)
        });
        let status = child.wait()?;
        Ok(Execution {
            code: status.code(),
            stdout: String::from_utf8_lossy(&stdout?).into_owned(),
            stderr: String::from_utf8_lossy(&stderr?).into_owned(),
        })
    }

    fn is_root(&self) -> bool {
        command::is_root()
    }

    fn has_command(&self, name: &str) -> bool {
        command::command_exists(name)
    }

    fn filesystem(&self) -> Option<&dyn Filesystem> {
        Some(&LocalFilesystem)
    }
}

/// Reads `source` to the end, copying each chunk to `echo` as it arrives.
fn drain(source: Option<impl Read>, mut echo: Option<impl Write>) -> io::Result<Vec<u8>> {
    let mut captured = Vec::new();
    let Some(mut source) = source else {
        return Ok(captured);
    };
    let mut buffer = [0_u8; 8192];
    loop {
        let read = match source.read(&mut buffer) {
            Ok(0) => return Ok(captured),
            Ok(read) => read,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err),
        };
        let chunk = &buffer[..read];
        // A closed terminal stops the echo, never the upgrade underneath it.
        if let Some(out) = echo.as_mut()
            && out.write_all(chunk).and_then(|()| out.flush()).is_err()
        {
            echo = None;
        }
        captured.extend_from_slice(chunk);
    }
}

#[cfg(test)]
mod tests;
