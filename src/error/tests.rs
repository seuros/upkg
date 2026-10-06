use super::*;

#[test]
fn usage_error_preserves_rendered_diagnostic() {
    let message = "error: invalid command\n\nFor more information, try '--help'.\n";
    let err = UpkgError::Usage {
        message: message.into(),
        code: 2,
    };
    assert_eq!(err.to_string(), message);
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
    let err = UpkgError::Usage {
        message: "test error".into(),
        code: 2,
    };
    let debug = format!("{:?}", err);
    assert!(debug.contains("Usage"));
}
