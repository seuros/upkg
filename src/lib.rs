//! upkg as a library.
//!
//! With the `host` feature, [`host`] keeps a Linux or FreeBSD machine
//! current: refresh metadata, upgrade everything (kernels and base system
//! included), clean up, and tell whether a reboot is due. [`manifest`] reads
//! a project's `upkg.toml` and resolves it through the [`catalog`] into the
//! native names one backend installs, with a stable key for that set;
//! [`host::Host::missing`] says which of them a host lacks, and
//! [`host::Host::install`] installs them. It needs nothing beyond std, serde
//! and a TOML parser:
//!
//! ```toml
// x-release-please-start-version
//! upkg = { version = "0.20.0", default-features = false, features = ["host"] }
// x-release-please-end
//! ```
//!
//! The default `cli` feature builds the `upkg` binary on top of it.

#[cfg(feature = "host")]
pub mod catalog;
#[cfg(feature = "host")]
pub mod command;
#[cfg(feature = "host")]
pub mod host;
#[cfg(feature = "host")]
pub mod manifest;
