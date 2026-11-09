use std::path::PathBuf;

use super::*;
use rstest::rstest;

fn invalid(text: &str) -> String {
    match text.parse::<Manifest>() {
        Err(Error::Invalid {
            path: None,
            message,
        }) => message,
        other => panic!("{text:?} must be invalid, got {other:?}"),
    }
}

/// A scratch file under the system temp dir, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str, text: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("upkg-manifest-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE_NAME);
        std::fs::write(&path, text).unwrap();
        Self(path)
    }

    fn dir(&self) -> &Path {
        self.0.parent().unwrap()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if let Some(dir) = self.0.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

#[test]
fn packages_keep_file_order() {
    let manifest: Manifest = "packages = [\"protobuf-compiler\", \"pkg-config\"]\n"
        .parse()
        .unwrap();
    assert_eq!(manifest.packages(), ["protobuf-compiler", "pkg-config"]);
}

#[test]
fn an_empty_list_is_a_manifest() {
    let manifest: Manifest = "packages = []\n".parse().unwrap();
    assert_eq!(manifest.packages(), Vec::<String>::new());
}

#[rstest]
#[case::unknown_key(
    "packages = []\npakages = [\"git\"]\n",
    "line 2, column 1: unknown key `pakages`; the only key is `packages`"
)]
#[case::future_table(
    "packages = []\n[freebsd]\npackages = [\"git\"]\n",
    "line 2, column 2: unknown key `freebsd`"
)]
#[case::missing_packages("", "missing key `packages`")]
#[case::not_a_list(
    "packages = \"git\"\n",
    "line 1, column 12: `packages` must be a list of package names"
)]
#[case::not_strings(
    "packages = [\"git\", 1]\n",
    "line 1, column 20: `packages[1]` must be a string"
)]
#[case::not_toml("packages = [\"git\"\n", "line 1, column 18: unclosed array")]
#[case::empty_name("packages = [\"\"]\n", "`packages[0]` is empty")]
#[case::option(
    "packages = [\"--allow-unauthenticated\"]\n",
    "`packages[0]` starts with `-`"
)]
#[case::two_words("packages = [\"git curl\"]\n", "whitespace")]
#[case::newline("packages = [\"git\\n\"]\n", "whitespace")]
fn parse_errors_say_where_and_what(#[case] text: &str, #[case] expected: &str) {
    let message = invalid(text);
    assert!(message.contains(expected), "{message}");
}

#[rstest]
#[case::wrong_type("packages = \"SECRET\"\n")]
#[case::bad_name("packages = [\"-SECRET\"]\n")]
#[case::spaced_name("packages = [\"SECRET word\"]\n")]
#[case::unknown_quoted_key("\"SECRET\\u001b[31m\" = 1\npackages = []\n")]
#[case::long_key(&format!("SECRET{} = 1\npackages = []\n", "x".repeat(80)))]
#[case::syntax("packages = [\"git\"]\nSECRET SECRET\n")]
#[case::unterminated("packages = [\"SECRET\n")]
fn parse_errors_never_quote_the_file(#[case] text: &str) {
    let message = invalid(text);
    assert!(!message.contains("SECRET"), "{message}");
    assert!(message.starts_with("line "), "{message}");
}

#[test]
fn load_names_the_file_in_errors() {
    let scratch = Scratch::new("invalid", "packages = [\"git\"]\nextra = true\n");
    let error = Manifest::load(&scratch.0).unwrap_err();
    let Error::Invalid {
        path: Some(path), ..
    } = &error
    else {
        panic!("expected Error::Invalid with a path, got {error:?}");
    };
    assert_eq!(path, &scratch.0);
    let shown = error.to_string();
    assert!(
        shown.starts_with(&scratch.0.display().to_string()),
        "{shown}"
    );
    assert!(shown.contains("unknown key `extra`"), "{shown}");
}

#[test]
fn load_reads_a_file() {
    let scratch = Scratch::new("valid", "packages = [\"git\"]\n");
    assert_eq!(Manifest::load(&scratch.0).unwrap().packages(), ["git"]);
}

#[test]
fn a_missing_file_is_a_read_error_saying_not_found() {
    let path = std::env::temp_dir().join("upkg-manifest-nonesuch/upkg.toml");
    let Err(Error::Read {
        path: shown,
        source,
    }) = Manifest::load(&path)
    else {
        panic!("a missing file must be Error::Read");
    };
    assert_eq!(shown, path);
    assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
    assert!(Manifest::load(&path).unwrap_err().is_not_found());
}

#[test]
fn load_refuses_a_file_past_the_cap() {
    let big = format!("packages = []\n#{}\n", "x".repeat(65 * 1024));
    let scratch = Scratch::new("load-big", &big);
    let error = Manifest::load(&scratch.0).unwrap_err();
    assert!(matches!(error, Error::Refused { .. }), "{error:?}");
    assert!(!error.is_not_found());
}

#[test]
fn load_from_dir_reads_the_dirs_upkg_toml() {
    let scratch = Scratch::new("dir-valid", "packages = [\"git\"]\n");
    let manifest = Manifest::load_from_dir(scratch.dir()).unwrap().unwrap();
    assert_eq!(manifest.packages(), ["git"]);
}

#[test]
fn load_from_dir_without_a_manifest_is_none() {
    let scratch = Scratch::new("dir-none", "");
    std::fs::remove_file(&scratch.0).unwrap();
    assert!(Manifest::load_from_dir(scratch.dir()).unwrap().is_none());
}

#[test]
fn load_from_dir_with_an_empty_list_is_an_empty_manifest() {
    let scratch = Scratch::new("dir-empty", "packages = []\n");
    let manifest = Manifest::load_from_dir(scratch.dir()).unwrap().unwrap();
    assert_eq!(manifest.packages(), Vec::<String>::new());
}

#[cfg(unix)]
#[test]
fn load_from_dir_never_follows_a_symlink() {
    let target = Scratch::new("dir-symlink-target", "packages = [\"git\"]\n");
    let scratch = Scratch::new("dir-symlink", "");
    std::fs::remove_file(&scratch.0).unwrap();
    std::os::unix::fs::symlink(&target.0, &scratch.0).unwrap();
    let error = Manifest::load_from_dir(scratch.dir()).unwrap_err();
    let Error::Refused { path, reason } = &error else {
        panic!("a symlink must be refused, got {error:?}");
    };
    assert_eq!(path, &scratch.0);
    assert!(reason.contains("not a regular file"), "{reason}");
    // Through `load`, a path someone chose, the same link is followed.
    assert_eq!(Manifest::load(&scratch.0).unwrap().packages(), ["git"]);
}

#[test]
fn load_from_dir_refuses_a_directory() {
    let scratch = Scratch::new("dir-dir", "");
    std::fs::remove_file(&scratch.0).unwrap();
    std::fs::create_dir(&scratch.0).unwrap();
    let error = Manifest::load_from_dir(scratch.dir()).unwrap_err();
    assert!(matches!(error, Error::Refused { .. }), "{error:?}");
}

#[test]
fn load_from_dir_refuses_a_file_past_the_cap() {
    let big = format!("packages = []\n#{}\n", "x".repeat(65 * 1024));
    let scratch = Scratch::new("dir-big", &big);
    let error = Manifest::load_from_dir(scratch.dir()).unwrap_err();
    let Error::Refused { reason, .. } = &error else {
        panic!("a huge file must be refused, got {error:?}");
    };
    assert!(reason.contains("64 KiB"), "{reason}");
}

#[test]
fn load_from_dir_refuses_text_that_is_not_utf8() {
    let scratch = Scratch::new("dir-binary", "");
    std::fs::write(&scratch.0, b"packages = [\"\xff\"]\n").unwrap();
    let error = Manifest::load_from_dir(scratch.dir()).unwrap_err();
    assert_eq!(error.detail(), "not UTF-8 text");
}

#[test]
fn load_from_dir_errors_name_the_file_and_say_where() {
    let scratch = Scratch::new("dir-invalid", "packages = [\"git\"]\nextra = \"SECRET\"\n");
    let error = Manifest::load_from_dir(scratch.dir()).unwrap_err();
    assert_eq!(
        error.to_string(),
        format!(
            "{}: line 2, column 1: unknown key `extra`; the only key is `packages`",
            scratch.0.display()
        )
    );
    assert_eq!(
        error.detail(),
        "line 2, column 1: unknown key `extra`; the only key is `packages`"
    );
}

#[rstest]
#[case("apt", &["pkg-config", "protobuf-compiler"])]
#[case("dnf", &["pkgconf-pkg-config", "protobuf-compiler"])]
#[case("pacman", &["pkgconf", "protobuf"])]
#[case("zypper", &["pkgconf-pkg-config", "protobuf-devel"])]
#[case("apk", &["pkgconf", "protoc"])]
#[case("freebsd", &["pkgconf", "protobuf"])]
#[case("brew", &["pkgconf", "protobuf"])]
// Unverified names fall through verbatim.
#[case("opkg", &["pkg-config", "protobuf-compiler"])]
fn resolve_gives_native_names_per_backend(#[case] backend: &str, #[case] expected: &[&str]) {
    let manifest: Manifest = "packages = [\"protobuf-compiler\", \"pkg-config\"]\n"
        .parse()
        .unwrap();
    let set = manifest.resolve(backend);
    assert_eq!(set.backend(), backend);
    assert_eq!(set.packages(), expected);
}
