#[cfg(target_os = "linux")]
#[path = "patch/linux.rs"]
pub mod linux;

#[cfg(target_os = "macos")]
#[path = "patch/macos.rs"]
pub mod macos;

#[cfg(unix)]
#[path = "patch/utils.rs"]
pub(crate) mod utils;
