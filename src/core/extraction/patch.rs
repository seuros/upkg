#[cfg(target_os = "macos")]
#[path = "patch/macos.rs"]
pub mod macos;

#[cfg(unix)]
#[path = "patch/utils.rs"]
pub(crate) mod utils;
