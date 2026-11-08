use std::fmt;
use std::io;

use super::Outcome;

/// What went wrong while maintaining the host.
#[derive(Debug)]
pub enum Error {
    /// This platform or manager can't do what was asked.
    Unsupported(&'static str),
    /// A command could not be started, or a file could not be read.
    Io {
        /// The command or path involved.
        context: String,
        source: io::Error,
    },
    /// A step exited unsuccessfully. The outcome holds every step so far,
    /// the failed one last, with its exit status and output tails.
    Failed {
        command: String,
        code: Option<i32>,
        outcome: Box<Outcome>,
    },
    /// The reboot check could not reach an answer.
    Probe(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(message) => write!(f, "{message}"),
            Self::Io { context, source } => write!(f, "`{context}`: {source}"),
            Self::Failed {
                command,
                code: Some(code),
                ..
            } => write!(f, "`{command}` exited with status code {code}"),
            Self::Failed { command, .. } => write!(f, "`{command}` was killed by a signal"),
            Self::Probe(message) => write!(f, "reboot check failed: {message}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
