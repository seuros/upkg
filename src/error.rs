use std::fmt;

#[derive(Debug)]
pub enum UpkgError {
    Usage {
        message: String,
        code: u8,
    },
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    Unsupported(&'static str),
    Io(std::io::Error),
    SelfUpgrade(String),
    #[cfg(target_os = "macos")]
    Native(crate::types::Error),
    #[cfg(not(target_os = "macos"))]
    CommandFailed {
        command: String,
        code: i32,
    },
    /// From the host-maintenance library.
    Host(upkg::host::Error),
    /// An upkg.toml that can't be read or isn't valid.
    Manifest(upkg::manifest::Error),
}

impl fmt::Display for UpkgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage { message, .. } => write!(f, "{message}"),
            Self::Unsupported(msg) => write!(f, "{msg}"),
            Self::Io(err) => write!(f, "{err}"),
            Self::SelfUpgrade(msg) => write!(f, "{msg}"),
            #[cfg(target_os = "macos")]
            Self::Native(err) => write!(f, "{err}"),
            #[cfg(not(target_os = "macos"))]
            Self::CommandFailed { command, code } => {
                write!(f, "`{command}` exited with status code {code}")
            }
            Self::Host(err) => write!(f, "{err}"),
            Self::Manifest(err) => write!(f, "{err}"),
        }
    }
}

impl From<upkg::host::Error> for UpkgError {
    fn from(value: upkg::host::Error) -> Self {
        match value {
            upkg::host::Error::Unsupported(message) => Self::Unsupported(message),
            other => Self::Host(other),
        }
    }
}

impl From<upkg::manifest::Error> for UpkgError {
    fn from(value: upkg::manifest::Error) -> Self {
        Self::Manifest(value)
    }
}

impl From<std::io::Error> for UpkgError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

#[cfg(test)]
mod tests;
