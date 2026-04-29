#[path = "installer/cask.rs"]
mod cask;
#[cfg(test)]
#[path = "installer/homebrew.rs"]
pub mod homebrew;
#[path = "installer/install.rs"]
pub mod install;
