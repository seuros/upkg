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
        }
    }
}

impl From<std::io::Error> for UpkgError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

#[cfg(test)]
mod tests;
