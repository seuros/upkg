use super::*;

#[test]
fn from_defaults_sets_expected_paths() {
    let context = Context::from_defaults();

    #[cfg(target_arch = "aarch64")]
    let root = PathBuf::from("/opt/homebrew");
    #[cfg(target_arch = "x86_64")]
    let root = PathBuf::from("/usr/local");

    assert_eq!(context.paths.root, root.clone());
    assert_eq!(context.paths.store, root.join("store"));
    assert_eq!(context.paths.cellar, root.join("Cellar"));
    assert_eq!(context.paths.cache, root.join("cache"));
    assert_eq!(context.paths.locks, root.join("locks"));
}
