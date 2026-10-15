use super::*;
use crate::core::cellar::link::test_support::write_keg;
use std::assert_matches;
use tempfile::TempDir;

fn steps() -> Vec<PostInstallStep> {
    serde_json::from_str(
        r#"[
          {"type":"mkdir_p","path":{"base":"var","path":"log"}},
          {"type":"link_dir","source":{"base":"prefix","path":"share/postgresql"},
           "target":{"base":"homebrew_prefix","path":"share/{{formula_name}}"}},
          {"type":"link_dir","source":{"base":"prefix","path":"lib/postgresql"},
           "target":{"base":"homebrew_prefix","path":"lib/{{formula_name}}"}},
          {"type":"link_children","source":{"base":"prefix","path":"bin"},
           "target":{"base":"homebrew_prefix","path":"bin"},"suffix":"-{{version.major}}"},
          {"type":"init_data_dir","path":{"base":"var","path":"{{formula_name}}"},
           "using":"postgresql_initdb"}
        ]"#,
    )
    .unwrap()
}

fn fixture(tmp: &TempDir, version: &str) -> (Linker, PathBuf) {
    let keg = write_keg(
        tmp.path(),
        &format!("postgresql@18/{version}"),
        &[
            ("share/postgresql/postgres.bki", b"bootstrap"),
            ("share/postgresql/extension/plpgsql.control", b"extension"),
            ("lib/postgresql/plpgsql.so", b"library"),
            ("bin/initdb", b"#!/bin/sh\nexit 1\n"),
        ],
    );
    (Linker::new(tmp.path()).unwrap(), keg)
}

/// Applies the postgres `steps()` plan to `keg`, which must succeed.
fn apply(linker: &Linker, keg: &Path) {
    linker.apply_post_install(keg, &steps()).unwrap();
}

/// Removes the postgres `steps()` plan for `keg`, which must succeed.
fn remove(linker: &Linker, keg: &Path) {
    linker.remove_post_install(keg, &steps()).unwrap();
}

#[test]
fn postgres_runtime_files_are_linked_without_global_binaries_or_database_initialization() {
    let tmp = TempDir::new().unwrap();
    let (linker, keg) = fixture(&tmp, "18.6");
    apply(&linker, &keg);
    assert_eq!(
        fs::read_to_string(tmp.path().join("share/postgresql@18/postgres.bki")).unwrap(),
        "bootstrap",
    );
    assert!(
        tmp.path()
            .join("share/postgresql@18/extension/plpgsql.control")
            .exists()
    );
    assert!(tmp.path().join("lib/postgresql@18/plpgsql.so").exists());
    assert!(tmp.path().join("bin/initdb-18").exists());
    assert!(!tmp.path().join("bin/initdb").exists());
    assert!(tmp.path().join("var/log").is_dir());
    assert!(!tmp.path().join("var/postgresql@18").exists());
    apply(&linker, &keg);
}

#[test]
fn upgrade_replaces_own_links_and_old_uninstall_does_not_remove_new_links() {
    let tmp = TempDir::new().unwrap();
    let (linker, old) = fixture(&tmp, "18.5");
    apply(&linker, &old);
    let (_, new) = fixture(&tmp, "18.6");
    apply(&linker, &new);
    remove(&linker, &old);
    assert_eq!(
        fs::canonicalize(tmp.path().join("share/postgresql@18/postgres.bki")).unwrap(),
        fs::canonicalize(new.join("share/postgresql/postgres.bki")).unwrap(),
    );
    assert!(tmp.path().join("bin/initdb-18").exists());
}

#[test]
fn unlink_preserves_extensions_and_user_data() {
    let tmp = TempDir::new().unwrap();
    let (linker, keg) = fixture(&tmp, "18.6");
    apply(&linker, &keg);
    let extra = tmp
        .path()
        .join("share/postgresql@18/extension/custom.control");
    fs::write(&extra, "user extension").unwrap();
    let data = tmp.path().join("var/postgresql@18");
    fs::create_dir_all(&data).unwrap();
    fs::write(data.join("PG_VERSION"), "18").unwrap();
    remove(&linker, &keg);
    assert!(!tmp.path().join("share/postgresql@18/postgres.bki").exists());
    assert!(!tmp.path().join("bin/initdb-18").exists());
    assert_eq!(fs::read_to_string(extra).unwrap(), "user extension");
    assert_eq!(fs::read_to_string(data.join("PG_VERSION")).unwrap(), "18");
}

#[test]
fn conflicting_runtime_file_rejects_the_whole_plan_without_overwriting() {
    let tmp = TempDir::new().unwrap();
    let (linker, keg) = fixture(&tmp, "18.6");
    let target = tmp.path().join("lib/postgresql@18/plpgsql.so");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, "user library").unwrap();
    assert_matches!(
        linker.apply_post_install(&keg, &steps()),
        Err(Error::LinkConflict { .. })
    );
    assert_eq!(fs::read_to_string(target).unwrap(), "user library");
    assert!(!tmp.path().join("share/postgresql@18/postgres.bki").exists());
    assert!(!tmp.path().join("var/log").exists());
}

#[test]
fn rejects_path_traversal_unknown_templates_and_escaping_symlinks() {
    let tmp = TempDir::new().unwrap();
    let (linker, keg) = fixture(&tmp, "18.6");
    for path in [
        "/tmp/escape",
        "../escape",
        "share/../../escape",
        "{{unknown}}",
        "",
    ] {
        let step = PostInstallStep::MkdirP {
            path: PostInstallPath {
                base: "homebrew_prefix".into(),
                path: path.into(),
            },
        };
        assert!(linker.apply_post_install(&keg, &[step]).is_err(), "{path}");
    }
    let external = TempDir::new().unwrap();
    std::os::unix::fs::symlink(external.path(), tmp.path().join("share/postgresql@18")).unwrap();
    assert!(linker.apply_post_install(&keg, &steps()).is_err());
    assert!(fs::read_dir(external.path()).unwrap().next().is_none());
}

#[test]
fn unknown_declarative_hooks_remain_non_executable() {
    let tmp = TempDir::new().unwrap();
    let (linker, keg) = fixture(&tmp, "18.6");
    let steps: Vec<PostInstallStep> =
        serde_json::from_str(r#"[{"type":"system","command":"touch /tmp/should-not-run"}]"#)
            .unwrap();
    assert_eq!(steps, vec![PostInstallStep::Unsupported]);
    linker.apply_post_install(&keg, &steps).unwrap();
}

#[test]
fn unlink_damaged_keg_removes_own_dangling_links_but_not_unrelated_dangling_links() {
    let tmp = TempDir::new().unwrap();
    let (linker, keg) = fixture(&tmp, "18.6");
    apply(&linker, &keg);
    let unrelated = tmp
        .path()
        .join("share/postgresql@18/extension/custom.control");
    std::os::unix::fs::symlink("/missing/unrelated-extension", &unrelated).unwrap();
    fs::remove_dir_all(keg.join("share")).unwrap();
    fs::remove_dir_all(keg.join("lib")).unwrap();
    fs::remove_dir_all(keg.join("bin")).unwrap();
    remove(&linker, &keg);
    assert!(
        !tmp.path()
            .join("share/postgresql@18/postgres.bki")
            .is_symlink()
    );
    assert!(!tmp.path().join("lib/postgresql@18/plpgsql.so").is_symlink());
    assert!(!tmp.path().join("bin/initdb-18").is_symlink());
    assert!(unrelated.is_symlink());
}

#[test]
fn templates_expand_in_one_pass_and_reject_unknown_or_unbalanced_markers() {
    let keg = Path::new("/p/Cellar/postgresql@18/18.6");
    let vars = KegVars::new(keg).unwrap();
    const CASES: &[(&str, Option<&str>)] = &[
        ("plain", Some("plain")),
        ("share/{{formula_name}}", Some("share/postgresql@18")),
        ("-{{version.major}}", Some("-18")),
        (
            "{{formula_name}}/{{version.major}}",
            Some("postgresql@18/18"),
        ),
        ("{{unknown}}", None),
        ("{{formula_name", None),
        ("a}}b", None),
        ("a}}{{formula_name}}", None),
        ("{{formula_name}}}}", None),
    ];
    for (input, expected) in CASES {
        assert_eq!(
            vars.expand(input).ok().as_deref(),
            *expected,
            "input: {input:?}"
        );
    }
    assert_matches!(vars.expand("plain"), Ok(Cow::Borrowed(_)));
}
