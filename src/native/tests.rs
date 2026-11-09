use super::*;

fn names(names: &[&str]) -> Vec<String> {
    names.iter().map(ToString::to_string).collect()
}

#[test]
fn formulae_resolve_through_the_catalog() {
    assert_eq!(
        resolve(
            &names(&["protobuf-compiler", "git", "libssl-dev"]),
            PackageKind::Auto
        ),
        ["protobuf", "git", "openssl@3"]
    );
}

#[test]
fn app_tokens_pass_as_given() {
    assert_eq!(
        resolve(&names(&["pkg-config"]), PackageKind::App),
        ["pkg-config"]
    );
}
