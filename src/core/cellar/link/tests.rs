use super::*;
use std::os::unix::fs::{PermissionsExt, symlink};
use tempfile::TempDir;
use test_support::write_keg;

/// Fresh prefix with a `Linker` over it.
fn linker() -> (TempDir, Linker) {
    let tmp = TempDir::new().unwrap();
    let linker = Linker::new(tmp.path()).unwrap();
    (tmp, linker)
}

fn make_executable(path: &Path) {
    fs::set_permissions(path, PermissionsExt::from_mode(0o755)).unwrap();
}

/// `<prefix>/Cellar/<name>/1.0.0` with one executable, `bin/<name>`.
fn setup_keg(prefix: &Path, name: &str) -> PathBuf {
    let exe = format!("bin/{name}");
    let keg = write_keg(prefix, &format!("{name}/1.0.0"), &[(&exe, b"hi")]);
    make_executable(&keg.join(exe));
    keg
}

fn setup_versioned_keg(prefix: &Path, name: &str, version: &str) -> PathBuf {
    let exe = format!("bin/{name}");
    let script = format!("#!/bin/sh\necho {name}-{version}");
    let header = format!("include/{name}/signature.h");
    let signature = format!("/* {name} {version} */");
    let keg = write_keg(
        prefix,
        &format!("{name}/{version}"),
        &[(&exe, script.as_bytes()), (&header, signature.as_bytes())],
    );
    make_executable(&keg.join(exe));
    keg
}

/// Two kegs whose `libexec/<alias>` symlinks to a sibling directory holding `files`.
fn write_aliased_kegs(
    prefix: &Path,
    kegs: [(&str, &str); 2],
    alias: &str,
    target: &str,
) -> [PathBuf; 2] {
    kegs.map(|(keg, file)| {
        let keg = write_keg(prefix, keg, &[(file, b"data")]);
        let link = keg.join("libexec").join(alias);
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        symlink(target, link).unwrap();
        keg
    })
}

#[test]
fn links_executables_to_bin() {
    let (tmp, linker) = linker();
    let keg = setup_keg(tmp.path(), "foo");
    linker.link_keg(&keg).unwrap();
    assert!(tmp.path().join("bin/foo").exists());
}

#[test]
fn merging_directories_works() {
    let (tmp, linker) = linker();
    let prefix = tmp.path();
    let keg1 = write_keg(prefix, "pkg1/1.0.0", &[("lib/pkgconfig/pkg1.pc", b"")]);
    let keg2 = write_keg(prefix, "pkg2/1.0.0", &[("lib/pkgconfig/pkg2.pc", b"")]);
    linker.link_keg(&keg1).unwrap();
    linker.link_keg(&keg2).unwrap();
    assert!(prefix.join("lib/pkgconfig/pkg1.pc").exists());
    assert!(prefix.join("lib/pkgconfig/pkg2.pc").exists());
}

#[test]
fn links_libexec_directory() {
    let (tmp, linker) = linker();
    let helper = "libexec/git-core/git-remote-https";
    let keg = write_keg(
        tmp.path(),
        "git/2.52.0",
        &[(helper, b"#!/bin/sh\necho helper")],
    );
    make_executable(&keg.join(helper));

    linker.link_keg(&keg).unwrap();

    let linked_helper = tmp.path().join(helper);
    assert!(linked_helper.exists(), "git-remote-https should be linked");
    assert!(linked_helper.is_symlink(), "should be a symlink");
}

#[test]
fn check_conflicts_passes_when_clean() {
    let (tmp, linker) = linker();
    let keg = setup_keg(tmp.path(), "foo");
    assert!(linker.check_conflicts(&keg).is_ok());
}

#[test]
fn check_conflicts_detects_conflicting_file() {
    let (tmp, linker) = linker();
    let prefix = tmp.path();

    let keg1 = setup_keg(prefix, "pkg1");
    linker.link_keg(&keg1).unwrap();

    let keg2 = write_keg(prefix, "pkg2/1.0.0", &[("bin/pkg1", b"conflict")]);
    make_executable(&keg2.join("bin/pkg1"));

    let result = linker.check_conflicts(&keg2);
    let Err(Error::LinkConflict { conflicts }) = result else {
        panic!("expected link conflict, got {result:?}");
    };
    assert_eq!(conflicts.len(), 1);
    assert!(conflicts[0].path.ends_with("bin/pkg1"));
    assert_eq!(conflicts[0].owned_by.as_deref(), Some("pkg1"));
}

#[test]
fn check_conflicts_collects_all_conflicts() {
    let (tmp, linker) = linker();
    let prefix = tmp.path();

    let keg1 = write_keg(
        prefix,
        "pkg1/1.0.0",
        &[("bin/tool-a", b"a"), ("bin/tool-b", b"b")],
    );
    linker.link_keg(&keg1).unwrap();

    let keg2 = write_keg(
        prefix,
        "pkg2/1.0.0",
        &[("bin/tool-a", b"x"), ("bin/tool-b", b"y")],
    );

    let result = linker.check_conflicts(&keg2);
    let Err(Error::LinkConflict { conflicts }) = result else {
        panic!("expected link conflict, got {result:?}");
    };
    assert_eq!(conflicts.len(), 2);
}

#[test]
fn link_keg_rejects_conflicts_without_creating_links() {
    let (tmp, linker) = linker();
    let prefix = tmp.path();

    let keg1 = setup_keg(prefix, "alpha");
    linker.link_keg(&keg1).unwrap();

    let keg2 = write_keg(
        prefix,
        "beta/1.0.0",
        &[("bin/alpha", b"other"), ("bin/beta-only", b"unique")],
    );

    assert!(linker.link_keg(&keg2).is_err());
    assert!(!prefix.join("bin/beta-only").exists());
    assert!(!prefix.join("opt/beta").exists());
}

#[test]
fn link_keg_replaces_links_owned_by_previous_version_of_same_formula() {
    let (tmp, linker) = linker();
    let prefix = tmp.path();

    let old_keg = setup_versioned_keg(prefix, "libgit2", "1.8.4");
    let new_keg = setup_versioned_keg(prefix, "libgit2", "1.9.3");

    linker.link_keg(&old_keg).unwrap();
    linker.link_keg(&new_keg).unwrap();

    for linked in ["bin/libgit2", "include/libgit2/signature.h"] {
        assert_eq!(
            fs::canonicalize(prefix.join(linked)).unwrap(),
            fs::canonicalize(new_keg.join(linked)).unwrap(),
            "{linked}"
        );
    }
}

#[test]
fn symlink_to_directory_in_keg_expands_without_conflict() {
    let (tmp, linker) = linker();
    let prefix = tmp.path();

    let [keg1, keg2] = write_aliased_kegs(
        prefix,
        [
            ("gnu-sed/4.9", "libexec/gnuman/man1/sed.1"),
            ("gnu-tar/1.35", "libexec/gnuman/man1/tar.1"),
        ],
        "gnubin/man",
        "../gnuman",
    );

    linker.link_keg(&keg1).unwrap();
    linker.link_keg(&keg2).unwrap();

    assert!(prefix.join("libexec/gnubin/man/man1/sed.1").exists());
    assert!(prefix.join("libexec/gnubin/man/man1/tar.1").exists());
    assert!(prefix.join("libexec/gnuman/man1/sed.1").exists());
    assert!(prefix.join("libexec/gnuman/man1/tar.1").exists());
}

#[test]
fn check_conflicts_passes_for_symlink_to_directory() {
    let (tmp, linker) = linker();

    let [keg1, keg2] = write_aliased_kegs(
        tmp.path(),
        [
            ("pkg1/1.0.0", "libexec/realdir/file1"),
            ("pkg2/1.0.0", "libexec/realdir/file2"),
        ],
        "alias",
        "realdir",
    );

    linker.link_keg(&keg1).unwrap();
    assert!(linker.check_conflicts(&keg2).is_ok());
}
