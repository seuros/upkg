use std::fmt;

#[derive(Debug)]
pub enum UpkgError {
    Usage(&'static str),
    #[cfg(any(
        target_os = "android",
        target_os = "linux",
        target_os = "windows",
        not(any(
            target_os = "android",
            target_os = "linux",
            target_os = "macos",
            target_os = "windows",
            target_os = "freebsd"
        ))
    ))]
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
            Self::Usage(msg) => write!(f, "{msg}\n\nTry `upkg help`."),
            #[cfg(any(
                target_os = "android",
                target_os = "linux",
                target_os = "windows",
                not(any(
                    target_os = "android",
                    target_os = "linux",
                    target_os = "macos",
                    target_os = "windows",
                    target_os = "freebsd"
                ))
            ))]
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
mod tests {
    use super::*;

    #[test]
    fn usage_error_displays_help_hint() {
        let err = UpkgError::Usage("invalid command");
        let msg = err.to_string();
        assert!(msg.contains("invalid command"));
        assert!(msg.contains("Try `upkg help`"));
    }

    #[test]
    fn io_error_wraps_correctly() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let err = UpkgError::from(io_err);
        let msg = err.to_string();
        assert!(msg.contains("file not found"));
    }

    #[test]
    fn self_upgrade_error_displays_message() {
        let err = UpkgError::SelfUpgrade("cannot self-upgrade".to_string());
        assert_eq!(err.to_string(), "cannot self-upgrade");
    }

    #[cfg(any(
        target_os = "android",
        target_os = "linux",
        target_os = "windows",
        not(any(
            target_os = "android",
            target_os = "linux",
            target_os = "macos",
            target_os = "windows",
            target_os = "freebsd"
        ))
    ))]
    #[test]
    fn unsupported_error_displays_message() {
        let err = UpkgError::Unsupported("unsupported platform");
        let msg = err.to_string();
        assert_eq!(msg, "unsupported platform");
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn command_failed_error_displays_command_and_code() {
        let err = UpkgError::CommandFailed {
            command: "sudo apt install -y git".to_string(),
            code: 1,
        };
        let msg = err.to_string();
        assert!(msg.contains("sudo apt install -y git"));
        assert!(msg.contains("1"));
        assert!(msg.contains("exited with status code"));
    }

    #[test]
    fn usage_error_is_debug_printable() {
        let err = UpkgError::Usage("test error");
        let debug = format!("{:?}", err);
        assert!(debug.contains("Usage"));
    }
}
