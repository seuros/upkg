//! upkg as a library.
//!
//! With the `host` feature, [`host`] keeps a Linux or FreeBSD machine
//! current: refresh metadata, upgrade everything (kernels and base system
//! included), clean up, and tell whether a reboot is due. It needs nothing
//! beyond std and serde:
//!
//! ```toml
//! upkg = { version = "0.19", default-features = false, features = ["host"] }
//! ```
//!
//! The default `cli` feature builds the `upkg` binary on top of it.

#[cfg(feature = "host")]
pub mod catalog;
#[cfg(feature = "host")]
pub mod command;
#[cfg(feature = "host")]
pub mod host;
