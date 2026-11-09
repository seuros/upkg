use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// An empty directory under the system temp dir, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("upkg-cli-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn with_manifest(self, text: &str) -> Self {
        std::fs::write(self.0.join("upkg.toml"), text).unwrap();
        self
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Runs upkg in `dir`, colours off, with any macOS engine state kept there.
fn upkg(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_upkg"))
        .args(args)
        .current_dir(dir)
        .env("UPKG_ROOT", dir)
        .env_remove("FORCE_COLOR")
        .env_remove("CLICOLOR_FORCE")
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[test]
fn help_uses_stdout_and_succeeds() {
    let scratch = Scratch::new("help");
    for args in [
        vec!["--help"],
        vec!["help"],
        vec!["install", "--help"],
        vec!["help", "install"],
        vec!["--self-upgrade", "--help"],
    ] {
        let output = upkg(&scratch.0, &args);
        assert!(output.status.success(), "{args:?}: {output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let text = text(&output.stdout);
        assert!(text.contains("Usage: upkg"), "{text}");
        if args.contains(&"install") || args.contains(&"--self-upgrade") {
            assert!(text.contains("--dry-run"), "{text}");
        }
    }
}

#[test]
fn version_output_is_unchanged() {
    let scratch = Scratch::new("version");
    for flag in ["--version", "-V"] {
        let output = upkg(&scratch.0, &[flag]);
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        assert_eq!(
            text(&output.stdout),
            format!("upkg {}\n", env!("CARGO_PKG_VERSION"))
        );
    }
}

#[test]
fn invalid_input_uses_stderr_and_fails() {
    // No upkg.toml here, so a bare `install` has nothing to install.
    let scratch = Scratch::new("invalid-input");
    for args in [
        vec![],
        vec!["install"],
        vec!["check"],
        vec!["nonesuch"],
        vec!["list", "extra"],
    ] {
        let output = upkg(&scratch.0, &args);
        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
        let text = String::from_utf8(output.stderr).unwrap();
        if args.is_empty() {
            // usage-rs explains a missing required subcommand with root help.
            assert!(text.contains("Usage: upkg"), "{text}");
        } else {
            assert_eq!(text.matches("error:").count(), 1, "{text}");
        }
        assert!(text.contains("--help"), "{text}");
    }
}

const MANIFEST: &str = "packages = [\"protobuf-compiler\", \"pkg-config\"]\n";

/// The catalog key behind the backend a dry run names.
#[cfg(not(windows))]
fn catalog_key(stdout: &str) -> String {
    if stdout.starts_with("engine: built-in macOS") {
        return "brew".to_string();
    }
    let backend = stdout
        .lines()
        .find_map(|line| line.strip_prefix("backend: "))
        .unwrap_or_else(|| panic!("no backend line in {stdout}"));
    match backend {
        "pkg" if cfg!(target_os = "android") => "termux".to_string(),
        "pkg" => "freebsd".to_string(),
        other => other.to_string(),
    }
}

/// A dry-run install planned the manifest's packages, resolved for this host.
#[cfg(not(windows))]
fn assert_plans_the_manifest(output: &Output) {
    let stdout = text(&output.stdout);
    assert!(output.status.success(), "{output:?}");
    let wanted = ["protobuf-compiler".to_string(), "pkg-config".to_string()];
    let native = upkg::catalog::resolve(&catalog_key(&stdout), &wanted).join(" ");
    let planned = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("dry-run: "))
        .next_back()
        .unwrap_or_else(|| panic!("no dry-run line in {stdout}"));
    assert!(
        planned.ends_with(&native),
        "{planned:?} should end with {native:?}"
    );
}

#[cfg(not(windows))]
#[test]
fn install_without_names_reads_upkg_toml() {
    let scratch = Scratch::new("install-manifest").with_manifest(MANIFEST);
    assert_plans_the_manifest(&upkg(&scratch.0, &["install", "--dry-run"]));
}

#[cfg(not(windows))]
#[test]
fn install_reads_the_manifest_it_is_pointed_at() {
    let project = Scratch::new("install-pointed").with_manifest(MANIFEST);
    let elsewhere = Scratch::new("install-elsewhere");
    let path = project.0.join("upkg.toml");
    let output = upkg(
        &elsewhere.0,
        &["install", "--dry-run", "--manifest", path.to_str().unwrap()],
    );
    assert_plans_the_manifest(&output);
}

#[test]
fn a_manifest_is_never_looked_for_above_the_working_directory() {
    let project = Scratch::new("no-search-up").with_manifest(MANIFEST);
    let below = project.0.join("crate");
    std::fs::create_dir_all(&below).unwrap();
    let output = upkg(&below, &["install", "--dry-run"]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(text(&output.stderr).contains("no upkg.toml"), "{output:?}");
}

#[test]
fn an_invalid_manifest_fails_before_touching_any_backend() {
    let scratch = Scratch::new("invalid-manifest").with_manifest("pakages = [\"git\"]\n");
    for args in [&["install", "--dry-run"][..], &["check"]] {
        let output = upkg(&scratch.0, args);
        assert_eq!(output.status.code(), Some(1), "{args:?}: {output:?}");
        assert!(output.stdout.is_empty(), "{args:?}: {output:?}");
        let stderr = text(&output.stderr);
        assert!(stderr.starts_with("error: upkg.toml: "), "{stderr}");
        assert!(
            stderr.contains("line 1, column 1: unknown key `pakages`"),
            "{stderr}"
        );
    }
}

#[cfg(unix)]
#[test]
fn a_symlinked_manifest_is_refused_before_touching_any_backend() {
    let elsewhere = Scratch::new("symlink-target").with_manifest("packages = [\"git\"]\n");
    let scratch = Scratch::new("symlink-manifest");
    std::os::unix::fs::symlink(elsewhere.0.join("upkg.toml"), scratch.0.join("upkg.toml")).unwrap();
    for args in [&["install", "--dry-run"][..], &["check"]] {
        let output = upkg(&scratch.0, args);
        assert_eq!(output.status.code(), Some(1), "{args:?}: {output:?}");
        assert!(output.stdout.is_empty(), "{args:?}: {output:?}");
        let stderr = text(&output.stderr);
        assert!(
            stderr.starts_with("error: upkg.toml: is not a regular file"),
            "{stderr}"
        );
    }
}

#[test]
fn an_empty_manifest_installs_nothing_and_says_so() {
    let scratch = Scratch::new("empty-manifest").with_manifest("packages = []\n");
    let output = upkg(&scratch.0, &["install"]);
    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert_eq!(text(&output.stderr), "note: upkg.toml lists no packages\n");

    let output = upkg(&scratch.0, &["check"]);
    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
}

// winget and choco can't answer `check`.
#[cfg(not(windows))]
#[test]
fn check_without_names_reports_what_the_manifest_misses() {
    let missing = "upkg-test-no-such-package";
    let scratch =
        Scratch::new("check-manifest").with_manifest(&format!("packages = [\"{missing}\"]\n"));
    let output = upkg(&scratch.0, &["check"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(
        text(&output.stdout),
        format!("missing\t{missing}\t{missing}\n")
    );
}
