use super::*;
use rstest::rstest;

fn names(names: &[&str]) -> Vec<String> {
    names.iter().map(ToString::to_string).collect()
}

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
#[case::unknown_key("packages = []\npakages = [\"git\"]\n", "unknown field `pakages`")]
#[case::future_table(
    "packages = []\n[freebsd]\npackages = [\"git\"]\n",
    "unknown field `freebsd`"
)]
#[case::missing_packages("", "missing field `packages`")]
#[case::not_a_list("packages = \"git\"\n", "invalid type")]
#[case::not_strings("packages = [1]\n", "invalid type")]
#[case::not_toml("packages = [\"git\"\n", "TOML parse error")]
#[case::empty_name("packages = [\"\"]\n", "is empty")]
#[case::option("packages = [\"--allow-unauthenticated\"]\n", "starts with `-`")]
#[case::two_words("packages = [\"git curl\"]\n", "whitespace")]
#[case::newline("packages = [\"git\\n\"]\n", "whitespace")]
fn parse_errors_say_what_is_wrong(#[case] text: &str, #[case] expected: &str) {
    let message = invalid(text);
    assert!(message.contains(expected), "{message}");
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
    assert!(shown.contains("unknown field `extra`"), "{shown}");
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
    assert_eq!(source.kind(), io::ErrorKind::NotFound);
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

#[test]
fn unknown_names_pass_through_verbatim() {
    let set = PackageSet::resolve("pacman", &names(&["ripgrep", "libpq-dev"]));
    assert_eq!(set.packages(), ["postgresql-libs", "ripgrep"]);
}

#[test]
fn key_is_backend_colon_sorted_unique_native_names() {
    let set = PackageSet::resolve("freebsd", &names(&["protobuf-compiler", "pkg-config"]));
    assert_eq!(set.key(), "freebsd:pkgconf protobuf");
}

#[test]
fn key_ignores_order_duplicates_and_spelling() {
    let written = PackageSet::resolve(
        "freebsd",
        &names(&["protobuf-compiler", "pkg-config", "protobuf-compiler"]),
    );
    // Native names, in another order, and both spellings of pkgconf.
    let other = PackageSet::resolve("freebsd", &names(&["protobuf", "pkgconf", "pkg-config"]));
    assert_eq!(written.key(), other.key());
    assert_eq!(written, other);
}

#[test]
fn key_differs_per_backend() {
    let wanted = names(&["git"]);
    assert_eq!(PackageSet::resolve("apt", &wanted).key(), "apt:git");
    assert_ne!(
        PackageSet::resolve("apt", &wanted).key(),
        PackageSet::resolve("dnf", &wanted).key()
    );
}

#[test]
fn key_of_nothing_is_the_backend_alone() {
    let set = PackageSet::resolve("apk", &[]);
    assert!(set.is_empty());
    assert_eq!(set.key(), "apk:");
}

#[test]
fn package_set_round_trips_through_json() {
    let set = PackageSet::resolve("freebsd", &names(&["protobuf-compiler", "pkg-config"]));
    let json = serde_json::to_string(&set).unwrap();
    assert_eq!(
        json,
        r#"{"backend":"freebsd","packages":["pkgconf","protobuf"]}"#
    );
    assert_eq!(serde_json::from_str::<PackageSet>(&json).unwrap(), set);
}

#[test]
fn deserializing_normalizes_like_resolving() {
    let set: PackageSet =
        serde_json::from_str(r#"{"backend":"apt","packages":["protobuf-compiler","git","git"]}"#)
            .unwrap();
    assert_eq!(set.key(), "apt:git protobuf-compiler");
}

#[rstest]
#[case::option_as_name(r#"{"backend":"apt","packages":["-oAPT::Get::AllowUnauthenticated=1"]}"#)]
#[case::spaced_name(r#"{"backend":"apt","packages":["git curl"]}"#)]
#[case::empty_backend(r#"{"backend":"","packages":["git"]}"#)]
#[case::colon_backend(r#"{"backend":"apt:git","packages":[]}"#)]
#[case::unknown_field(r#"{"backend":"apt","packages":[],"extra":1}"#)]
fn deserializing_refuses_what_resolving_cannot_produce(#[case] json: &str) {
    assert!(serde_json::from_str::<PackageSet>(json).is_err(), "{json}");
}
