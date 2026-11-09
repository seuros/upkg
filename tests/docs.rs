//! The dependency line the docs hand out names the version being released.
//! release-please rewrites it (`extra-files` in release-please-config.json);
//! this catches a marker that went missing or a line added outside one.

use std::path::Path;

const START: &str = "x-release-please-start-version";
const END: &str = "x-release-please-end";

/// Every `upkg = { version = ... }` line in `file`, and whether it sits
/// between release-please's markers.
fn dependency_lines(file: &str) -> Vec<(String, bool)> {
    let text = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(file)).unwrap();
    let mut marked = false;
    let mut lines = Vec::new();
    for line in text.lines() {
        if line.contains(START) {
            marked = true;
        } else if line.contains(END) {
            marked = false;
        } else if line.contains("upkg = { version =") {
            lines.push((line.to_string(), marked));
        }
    }
    lines
}

#[test]
fn docs_ask_for_the_version_being_released() {
    let wanted = format!("version = \"{}\"", env!("CARGO_PKG_VERSION"));
    for file in ["README.md", "src/lib.rs"] {
        let lines = dependency_lines(file);
        assert!(!lines.is_empty(), "{file} shows no dependency line");
        for (line, marked) in lines {
            assert!(marked, "{file}: outside release-please's markers: {line}");
            assert!(line.contains(&wanted), "{file}: not {wanted}: {line}");
        }
    }
}
