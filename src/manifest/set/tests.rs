use super::*;
use rstest::rstest;

fn names(names: &[&str]) -> Vec<String> {
    names.iter().map(ToString::to_string).collect()
}

fn pairs(set: &PackageSet) -> Vec<(&str, &str)> {
    set.entries()
        .iter()
        .map(|entry| (entry.requested(), entry.native()))
        .collect()
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

#[test]
fn entries_keep_what_was_asked_for_next_to_the_native_name() {
    let set = PackageSet::resolve(
        "freebsd",
        &names(&["protobuf-compiler", "git", "protobuf-compiler"]),
    );
    assert_eq!(set.packages(), ["git", "protobuf"]);
    assert_eq!(
        pairs(&set),
        [("git", "git"), ("protobuf-compiler", "protobuf")]
    );
}

#[test]
fn two_spellings_of_one_package_are_two_entries_and_one_name() {
    let set = PackageSet::resolve("freebsd", &names(&["pkgconf", "pkg-config"]));
    assert_eq!(set.packages(), ["pkgconf"]);
    assert_eq!(
        pairs(&set),
        [("pkg-config", "pkgconf"), ("pkgconf", "pkgconf")]
    );
    assert_eq!(set.key(), "freebsd:pkgconf");
}

#[test]
fn a_package_shows_its_alias_only_when_there_is_one() {
    let set = PackageSet::resolve("freebsd", &names(&["protobuf-compiler", "git"]));
    let shown: Vec<String> = set.entries().iter().map(ToString::to_string).collect();
    assert_eq!(shown, ["git", "protobuf-compiler (protobuf)"]);
    assert!(!set.entries()[0].is_alias());
    assert!(set.entries()[1].is_alias());
}

#[test]
fn spelling_never_changes_equality_hash_or_key() {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let hash = |set: &PackageSet| {
        let mut hasher = DefaultHasher::new();
        set.hash(&mut hasher);
        hasher.finish()
    };
    let canonical = PackageSet::resolve("freebsd", &names(&["protobuf-compiler"]));
    let native = PackageSet::resolve("freebsd", &names(&["protobuf"]));
    assert_ne!(pairs(&canonical), pairs(&native));
    assert_eq!(canonical, native);
    assert_eq!(hash(&canonical), hash(&native));
    assert_eq!(canonical.key(), native.key());
}

#[test]
fn json_keeps_its_shape_and_entries_come_back_native() {
    let set = PackageSet::resolve("freebsd", &names(&["protobuf-compiler", "pkg-config"]));
    let json = serde_json::to_string(&set).unwrap();
    // Byte for byte what upkg 0.20 wrote, so either side of a process
    // boundary can be the older one.
    assert_eq!(
        json,
        r#"{"backend":"freebsd","packages":["pkgconf","protobuf"]}"#
    );
    let back: PackageSet = serde_json::from_str(&json).unwrap();
    assert_eq!(back, set);
    assert_eq!(
        pairs(&back),
        [("pkgconf", "pkgconf"), ("protobuf", "protobuf")]
    );
}

#[rstest]
// The values are FNV-1a 32 of the key, worked out independently.
#[case(&["protobuf-compiler", "pkg-config"], "freebsd:pkgconf protobuf", "3ce9857d")]
#[case(&["protobuf-compiler"], "freebsd:protobuf", "922d2bbd")]
#[case(&[], "freebsd:", "37efdcca")]
fn digest_is_eight_hex_digits_of_fnv1a_over_the_key(
    #[case] packages: &[&str],
    #[case] key: &str,
    #[case] digest: &str,
) {
    let set = PackageSet::resolve("freebsd", &names(packages));
    assert_eq!(set.key(), key);
    assert_eq!(set.digest(), digest);
}

#[test]
fn digest_follows_the_key_not_the_spelling() {
    let written = PackageSet::resolve("freebsd", &names(&["protobuf-compiler", "pkg-config"]));
    let native = PackageSet::resolve("freebsd", &names(&["pkgconf", "protobuf"]));
    assert_eq!(written.digest(), native.digest());
    assert_ne!(
        written.digest(),
        PackageSet::resolve("apt", &names(&["protobuf-compiler", "pkg-config"])).digest()
    );
}
