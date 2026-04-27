#[cfg(target_os = "linux")]
#[path = "patch/linux.rs"]
pub mod linux;

#[cfg(target_os = "macos")]
#[path = "patch/macos.rs"]
pub mod macos;

#[cfg(target_os = "linux")]
pub use linux::patch_placeholders;

#[cfg(target_os = "macos")]
pub use macos::{codesign_and_strip_xattrs, patch_homebrew_placeholders};
