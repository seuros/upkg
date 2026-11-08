//! The seam between upkg and the machine: commands, and the few files the
//! reboot check reads.

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

/// Runs commands and answers questions about the filesystem.
///
/// Implement [`Runner::run`] to fake command execution in tests. The other
/// methods default to the real system; override them to fake the reboot
/// check's files, root detection and `PATH` lookups as well.
pub trait Runner {
    /// Runs `spec` to completion and returns its exit status and output.
    ///
    /// # Errors
    ///
    /// The process could not be started or its output could not be read.
    fn run(&self, spec: &CommandSpec) -> io::Result<Execution>;

    /// Whether the effective user is root.
    fn is_root(&self) -> bool {
        command::is_root()
    }

    /// Whether `name` is an executable on `PATH`.
    fn has_command(&self, name: &str) -> bool {
        command::command_exists(name)
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    /// # Errors
    ///
    /// As [`fs::read_to_string`].
    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        fs::read_to_string(path)
    }

    /// When `path` was last modified.
    ///
    /// # Errors
    ///
    /// As [`fs::metadata`].
    fn modified(&self, path: &Path) -> io::Result<SystemTime> {
        fs::metadata(path)?.modified()
    }

    /// The names of the entries in `dir`.
    ///
    /// # Errors
    ///
    /// As [`fs::read_dir`].
    fn read_dir(&self, dir: &Path) -> io::Result<Vec<String>> {
        fs::read_dir(dir)?
            .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
            .collect()
    }
}

/// Runs real processes. stdin is closed so nothing can stop to ask a
/// question; stdout and stderr are captured, and echoed live when asked.
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
