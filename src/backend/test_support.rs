//! Shared assertions for per-platform backend tests.

use super::CommandSpec;
#[cfg(not(target_os = "windows"))]
use crate::error::UpkgError;

/// Asserts `spec` runs `command` with exactly `args`.
pub(super) fn assert_spec(spec: &CommandSpec, command: &str, args: &[&str]) {
    assert_eq!(spec.command(), command, "spec: {}", spec.render());
    assert_eq!(spec.args(), args, "spec: {}", spec.render());
}

/// pkg(8) and rvn share the `search -e <query>` exact-match contract.
/// Gated like `ravenports`, which compiles everywhere `freebsd` does.
#[cfg(not(target_os = "windows"))]
pub(super) fn assert_exact_search_uses_dash_e(
    search_spec: fn(&str, bool) -> Result<CommandSpec, UpkgError>,
) {
    let spec = search_spec("git", true).unwrap();
    assert_eq!(
        spec.args(),
        ["search", "-e", "git"],
        "spec: {}",
        spec.render()
    );
}
