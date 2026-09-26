use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn since_epoch() -> Duration {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
}

#[cfg(target_os = "macos")]
pub fn unix_secs() -> i64 {
    since_epoch().as_secs() as i64
}

pub fn unix_nanos() -> u128 {
    since_epoch().as_nanos()
}

pub fn unique_temp_dir(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "upkg-{label}-{}-{}",
        std::process::id(),
        unix_nanos()
    ))
}
