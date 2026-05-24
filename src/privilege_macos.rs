use std::process::Command;

pub fn escalate_privilege(shell_cmd: &str) -> Result<bool, String> {
    // Try macOS native authorization dialog first (works in GUI sessions)
    if has_gui_session() {
        let script = format!(
            "do shell script \"{}\" with administrator privileges",
            shell_cmd.replace('\\', "\\\\").replace('"', "\\\"")
        );
        let output = Command::new("osascript").args(["-e", &script]).output();

        if let Ok(output) = output {
            return Ok(output.status.success());
        }
    }

    // Fall back to sudo for SSH/headless sessions
    let output = Command::new("sudo")
        .args(["sh", "-c", shell_cmd])
        .status()
        .map_err(|e| format!("Failed to escalate privileges: {e}"))?;

    Ok(output.success())
}

fn has_gui_session() -> bool {
    Command::new("launchctl")
        .args(["managerpid"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
        && std::env::var("SSH_CONNECTION").is_err()
}

pub fn has_xcode_clt() -> bool {
    Command::new("xcode-select")
        .arg("-p")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn warn_if_xcode_clt_missing() {
    if !has_xcode_clt() {
        eprintln!(
            "    Warning: Xcode Command Line Tools not found; run `xcode-select --install` if Mach-O patching fails"
        );
    }
}
