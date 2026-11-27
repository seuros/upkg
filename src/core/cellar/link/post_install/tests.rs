use super::*;
use crate::core::cellar::link::test_support::write_keg;
use crate::types::formula::post_install::deserialize_steps;
use std::assert_matches;
use tempfile::TempDir;

/// Steps as a formula or receipt parses them.
fn parse(json: &str) -> Vec<PostInstallStep> {
    deserialize_steps(&mut serde_json::Deserializer::from_str(json)).unwrap()
}

fn steps() -> Vec<PostInstallStep> {
    parse(
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
fn rejects_path_traversal_and_unknown_templates() {
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
}

#[test]
fn unknown_declarative_hooks_remain_non_executable() {
    let tmp = TempDir::new().unwrap();
    let (linker, keg) = fixture(&tmp, "18.6");
    let steps = parse(r#"[{"type":"system","command":"touch /tmp/should-not-run"}]"#);
    assert_eq!(
        steps,
        vec![PostInstallStep::Unsupported {
            kind: Some("system".into())
        }]
    );
    linker.apply_post_install(&keg, &steps).unwrap();
}

#[rstest::rstest]
#[case("{{HOMEBREW_PREFIX}}/../escape")]
#[case("{{HOMEBREW_PREFIX}}/lib/../../escape")]
#[case("{{HOMEBREW_PREFIX}}//tmp/escape")]
#[case("{{HOMEBREW_PREFIX}}/{{unknown}}")]
#[case("{{HOMEBREW_PREFIX}}/")]
#[case("{{HOMEBREW_PREFIX}}_other/lib")]
#[case("/tmp/escape")]
#[case("{{unknown}}/lib")]
#[case("lib/gio/modules")]
#[case("")]
fn prefix_template_without_base_rejects_unsafe_paths(#[case] path: &str) {
    let tmp = TempDir::new().unwrap();
    let (linker, keg) = fixture(&tmp, "18.6");
    let steps: Vec<PostInstallStep> = serde_json::from_value(serde_json::json!([{
        "type": "mkdir_p",
        "path": { "path": path }
    }]))
    .unwrap();
    assert!(linker.apply_post_install(&keg, &steps).is_err());
    assert!(!tmp.path().join("lib/gio").exists());
    assert!(!tmp.path().join("escape").exists());
}

#[rstest::rstest]
#[case(false)]
#[case(true)]
fn rejects_escaping_directory_symlinks(#[case] prefix_template: bool) {
    let tmp = TempDir::new().unwrap();
    let (linker, keg) = fixture(&tmp, "18.6");
    let external = TempDir::new().unwrap();
    let (directory, steps) = if prefix_template {
        let formula: crate::Formula =
            serde_json::from_str(include_str!("../../../../fixtures/formula_glib.json")).unwrap();
        ("lib/gio", formula.post_install_steps)
    } else {
        ("share/postgresql@18", steps())
    };
    std::os::unix::fs::symlink(external.path(), tmp.path().join(directory)).unwrap();
    assert!(linker.apply_post_install(&keg, &steps).is_err());
    assert!(fs::read_dir(external.path()).unwrap().next().is_none());
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

/// openssl@4's step as Homebrew publishes it, against an existing
/// ca-certificates bundle.
fn openssl(tmp: &TempDir) -> (Linker, PathBuf, PathBuf, Vec<PostInstallStep>) {
    let keg = write_keg(tmp.path(), "openssl@4/4.0.3", &[("bin/openssl", b"binary")]);
    let bundle = tmp.path().join("etc/ca-certificates/cert.pem");
    fs::create_dir_all(bundle.parent().unwrap()).unwrap();
    fs::write(&bundle, "certificates").unwrap();
    let steps = parse(
        r#"[{"source":{"path":"{{etc}}/ca-certificates/cert.pem"},
             "target":{"path":"{{pkgetc}}/cert.pem"},"force":true,"type":"symlink"}]"#,
    );
    (Linker::new(tmp.path()).unwrap(), keg, bundle, steps)
}

#[test]
fn symlink_links_one_file_and_reapplies_over_itself() {
    let tmp = TempDir::new().unwrap();
    let (linker, keg, bundle, steps) = openssl(&tmp);
    let link = tmp.path().join("etc/openssl@4/cert.pem");
    for _ in 0..2 {
        linker.apply_post_install(&keg, &steps).unwrap();
        assert_eq!(fs::read_link(&link).unwrap(), bundle);
    }
    assert_eq!(fs::read_to_string(&link).unwrap(), "certificates");
}

#[test]
fn forced_symlink_replaces_a_file_but_never_a_directory() {
    let tmp = TempDir::new().unwrap();
    let (linker, keg, bundle, steps) = openssl(&tmp);
    let link = tmp.path().join("etc/openssl@4/cert.pem");
    fs::create_dir_all(link.parent().unwrap()).unwrap();
    fs::write(&link, "stale copy").unwrap();
    linker.apply_post_install(&keg, &steps).unwrap();
    assert_eq!(fs::read_link(&link).unwrap(), bundle);

    fs::remove_file(&link).unwrap();
    fs::create_dir(&link).unwrap();
    assert_matches!(
        linker.apply_post_install(&keg, &steps),
        Err(Error::LinkConflict { .. })
    );
    assert!(link.is_dir());
}

#[test]
fn unforced_symlink_conflicts_with_a_file_without_touching_it() {
    let tmp = TempDir::new().unwrap();
    let (linker, keg, _bundle, _) = openssl(&tmp);
    let steps = parse(
        r#"[{"source":{"path":"{{etc}}/ca-certificates/cert.pem"},
             "target":{"path":"{{pkgetc}}/cert.pem"},"type":"symlink"}]"#,
    );
    let link = tmp.path().join("etc/openssl@4/cert.pem");
    fs::create_dir_all(link.parent().unwrap()).unwrap();
    fs::write(&link, "user file").unwrap();
    assert_matches!(
        linker.apply_post_install(&keg, &steps),
        Err(Error::LinkConflict { .. })
    );
    assert_eq!(fs::read_to_string(link).unwrap(), "user file");
}

#[test]
fn removing_a_symlink_step_keeps_the_source_and_foreign_links() {
    let tmp = TempDir::new().unwrap();
    let (linker, keg, bundle, steps) = openssl(&tmp);
    let link = tmp.path().join("etc/openssl@4/cert.pem");
    linker.apply_post_install(&keg, &steps).unwrap();
    linker.remove_post_install(&keg, &steps).unwrap();
    assert!(link.symlink_metadata().is_err());
    assert_eq!(fs::read_to_string(&bundle).unwrap(), "certificates");

    std::os::unix::fs::symlink("/somewhere/else.pem", &link).unwrap();
    linker.remove_post_install(&keg, &steps).unwrap();
    assert!(link.is_symlink());
}

#[test]
fn glob_symlinks_and_unknown_steps_are_skipped_and_named() {
    let tmp = TempDir::new().unwrap();
    let (linker, keg, ..) = openssl(&tmp);
    let steps = parse(
        r#"[{"source":{"path":"{{HOMEBREW_PREFIX}}/lib/node_modules/npm/man/man1/*"},
             "target":{"path":"{{HOMEBREW_PREFIX}}/share/man/man1"},
             "force":true,"source_glob":true,"type":"symlink"},
            {"command":{"base":"libexec","path":"post-install"},"args":[],"type":"run"},
            {"type":"unsupported"}]"#,
    );
    linker.apply_post_install(&keg, &steps).unwrap();
    assert!(!tmp.path().join("share/man/man1").exists());
    let skipped: Vec<_> = steps.iter().filter_map(PostInstallStep::skipped).collect();
    assert_eq!(skipped, ["symlink (glob)", "run", "unknown"]);
}

#[test]
fn unsupported_steps_keep_their_kind_through_a_receipt() {
    let step = PostInstallStep::Unsupported {
        kind: Some("run".into()),
    };
    let json = serde_json::to_string(&[&step]).unwrap();
    assert_eq!(json, r#"[{"type":"unsupported","kind":"run"}]"#);
    assert_eq!(parse(&json), [step]);
}

#[test]
fn a_malformed_known_step_is_an_error_not_a_skip() {
    let json = r#"[{"source":{"path":"{{etc}}/x"},"type":"symlink"}]"#;
    assert!(deserialize_steps(&mut serde_json::Deserializer::from_str(json)).is_err());
}

#[rstest::rstest]
#[case("{{prefix}}/etc/ld.so.conf", "Cellar/glibc/2.41/etc/ld.so.conf")]
#[case(
    "{{pkgshare}}/certificates",
    "Cellar/glibc/2.41/share/glibc/certificates"
)]
#[case(
    "{{bash_completion}}/rustup",
    "Cellar/glibc/2.41/etc/bash_completion.d/rustup"
)]
#[case(
    "{{zsh_completion}}/_rustup",
    "Cellar/glibc/2.41/share/zsh/site-functions/_rustup"
)]
#[case(
    "{{fish_completion}}/r.fish",
    "Cellar/glibc/2.41/share/fish/vendor_completions.d/r.fish"
)]
#[case(
    "{{pwsh_completion}}/_r.ps1",
    "Cellar/glibc/2.41/share/pwsh/completions/_r.ps1"
)]
#[case("{{pkgetc}}/cert.pem", "etc/glibc/cert.pem")]
#[case("{{var}}/lib/i2pd/i2pd.conf", "var/lib/i2pd/i2pd.conf")]
#[case("{{HOMEBREW_PREFIX}}/bin/rustup", "bin/rustup")]
fn symlink_templates_follow_homebrew_path_helpers(#[case] path: &str, #[case] expected: &str) {
    let tmp = TempDir::new().unwrap();
    let keg = tmp.path().join("Cellar/glibc/2.41");
    let linker = Linker::new(tmp.path()).unwrap();
    let vars = KegVars::new(&keg).unwrap();
    let path = PostInstallPath {
        base: String::new(),
        path: path.into(),
    };
    assert_eq!(
        linker.helper_path(&vars, &path).unwrap(),
        tmp.path().join(expected)
    );
}

#[rstest::rstest]
#[case("{{etc}}")]
#[case("{{etc}}/")]
#[case("{{etc}}/../escape")]
#[case("{{unknown}}/x")]
#[case("/etc/passwd")]
#[case("{{etc}}/{{unknown}}")]
fn symlink_templates_reject_unsafe_paths(#[case] path: &str) {
    let tmp = TempDir::new().unwrap();
    let (linker, keg, ..) = openssl(&tmp);
    let steps = vec![PostInstallStep::Symlink {
        source: PostInstallPath {
            base: String::new(),
            path: "{{etc}}/ca-certificates/cert.pem".into(),
        },
        target: PostInstallPath {
            base: String::new(),
            path: path.into(),
        },
        force: true,
        source_glob: false,
    }];
    assert!(linker.apply_post_install(&keg, &steps).is_err(), "{path}");
}
