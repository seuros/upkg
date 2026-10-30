use super::*;
use crate::types::BuildSystem;
use std::assert_matches;
use std::fs;

fn plan(keg: &Path) -> BuildPlan {
    BuildPlan {
        formula_name: "coreutils".to_string(),
        version: "9.12_1".to_string(),
        source_url: "https://ftp.gnu.org/gnu/coreutils/coreutils-9.12.tar.xz".to_string(),
        source_checksum: Some(SOURCE_SHA256.to_string()),
        ruby_source_path: None,
        build_dependencies: vec!["texinfo".to_string()],
        runtime_dependencies: vec!["gmp".to_string(), "openssl@4".to_string()],
        detected_system: BuildSystem::Autoconf,
        prefix: keg.parent().unwrap().join("prefix"),
        cellar_path: keg.to_path_buf(),
    }
}

#[test]
fn accepts_reviewed_formula_and_bottle_rebuilds() {
    for checksum_or_url in [SOURCE_SHA256, PATCH_URL, PATCH_SHA256] {
        assert!(FORMULA.contains(checksum_or_url));
    }
    let plan = plan(Path::new("/tmp/coreutils"));
    validate(&plan, FORMULA).unwrap();
    let rebuilt = FORMULA.replace(
        "class Coreutils < Formula",
        "class Coreutils < Formula\n  bottle do\n    sha256 arm64_sonoma: \"new-bottle\"\n  end",
    );
    validate(&plan, &rebuilt).unwrap();
    validate(&plan, &FORMULA.replace("    system", "      system")).unwrap();
}

#[rstest::rstest]
#[case("--program-prefix=g", "--program-prefix=x")]
#[case("%w[dir dircolors vdir]", "%w[dir vdir]")]
#[case("type :backport", "type :other")]
#[case("deny_network_access!", "deny_network_access!\n  patch :DATA")]
#[case(
    "  def install",
    "  def install\n    bottle do\n      system \"unexpected\"\n    end"
)]
#[case(
    "  def install",
    "  def post_install\n    system \"unexpected\"\n  end\n  def install"
)]
fn rejects_changed_formula_logic(#[case] from: &str, #[case] to: &str) {
    let changed = FORMULA.replace(from, to);
    assert_ne!(changed, FORMULA);
    assert_matches!(
        validate(&plan(Path::new("/tmp/coreutils")), &changed),
        Err(Error::UnsupportedFormula { .. })
    );
}

#[test]
fn rejects_changed_source_version_and_invalid_syntax() {
    let mut plan = plan(Path::new("/tmp/coreutils"));
    plan.version = "9.13".to_string();
    assert!(validate(&plan, FORMULA).is_err());
    plan.version = "9.12_1".to_string();
    plan.source_checksum = None;
    assert!(validate(&plan, FORMULA).is_err());
    assert!(formula_tokens("class Coreutils < Formula\n def install").is_err());
}

#[test]
fn guard_compares_syntax_shape_not_just_lexemes() {
    assert_ne!(
        formula_tokens("x = foo\nbar").unwrap(),
        formula_tokens("x = foo bar").unwrap()
    );
}

fn fake_dependencies(root: &Path) -> HashMap<String, DepInfo> {
    let mut deps = HashMap::new();
    for (name, file) in [
        ("gmp", "include/gmp.h"),
        ("openssl@4", "include/openssl/ssl.h"),
        ("texinfo", "bin/makeinfo"),
    ] {
        let path = root.join(name);
        fs::create_dir_all(path.join(file).parent().unwrap()).unwrap();
        fs::write(path.join(file), "").unwrap();
        deps.insert(
            name.to_string(),
            DepInfo {
                cellar_path: path.to_str().unwrap().to_string(),
            },
        );
    }
    deps
}

#[test]
fn dependency_environment_uses_kegs_and_quotes_paths() {
    let tmp = tempfile::tempdir().unwrap();
    let deps = fake_dependencies(&tmp.path().join("dependencies with spaces"));
    check_dependencies(&deps).unwrap();
    let env: HashMap<_, _> = build_environment(&deps).unwrap().into_iter().collect();
    let paths: Vec<_> = std::env::split_paths(&env["PATH"]).collect();
    assert_eq!(paths[0], Path::new(&deps["gmp"].cellar_path).join("bin"));
    let flags = shlex::split(env["CPPFLAGS"].to_str().unwrap()).unwrap();
    assert_eq!(
        flags[0],
        format!(
            "-I{}",
            Path::new(&deps["gmp"].cellar_path)
                .join("include")
                .display()
        )
    );
    assert_eq!(env["CFLAGS"], "-O2");
    fs::remove_file(Path::new(&deps["texinfo"].cellar_path).join("bin/makeinfo")).unwrap();
    assert!(check_dependencies(&deps).is_err());
    assert!(check_dependencies(&HashMap::new()).is_err());
}

#[test]
fn rejects_broken_dependency_install_names() {
    let tmp = tempfile::tempdir().unwrap();
    let library = tmp.path().join("libgmp.10.dylib");
    fs::write(&library, "").unwrap();
    validate_library_id("gmp", library.to_str().unwrap()).unwrap();
    assert_matches!(
        validate_library_id("gmp", "/missing/opt/gmp/6.3.0/libgmp.10.dylib"),
        Err(Error::ExecutionError { message }) if message.contains("reinstall or repair")
    );
    assert!(validate_library_id("gmp", "").is_err());
}

fn installed_files(keg: &Path) {
    fs::create_dir_all(keg.join("bin")).unwrap();
    fs::create_dir_all(keg.join("share/man/man1")).unwrap();
    for name in ["gcat", "gdir", "gdircolors", "gvdir", "gshuf", "ggroups"] {
        fs::write(keg.join("bin").join(name), name).unwrap();
        fs::write(keg.join("share/man/man1").join(format!("{name}.1")), name).unwrap();
    }
    fs::write(keg.join("bin/.DS_Store"), "").unwrap();
}

#[test]
fn compatibility_links_are_relative_and_do_not_shadow_system_commands() {
    let tmp = tempfile::tempdir().unwrap();
    let keg = tmp.path().join("keg");
    installed_files(&keg);
    create_compatibility_links(&keg).unwrap();
    for name in ["cat", "dir", "dircolors", "vdir", "shuf", "groups"] {
        assert!(keg.join("libexec/gnubin").join(name).is_file(), "{name}");
        assert!(
            keg.join("libexec/gnuman/man1")
                .join(format!("{name}.1"))
                .is_file(),
            "{name}"
        );
    }
    assert!(!keg.join("bin/cat").exists());
    assert!(!keg.join("bin/dir").exists());
    assert!(!keg.join("bin/dircolors").exists());
    assert!(!keg.join("bin/vdir").exists());
    assert!(keg.join("bin/shuf").exists());
    assert!(keg.join("share/man/man1/shuf.1").exists());
    assert!(keg.join("libexec/gnubin/man/man1/cat.1").is_file());
    assert!(!keg.join("libexec/gnubin/roups").exists());
    let relocated = tmp.path().join("relocated");
    fs::rename(&keg, &relocated).unwrap();
    assert!(relocated.join("libexec/gnubin/cat").is_file());
    assert!(relocated.join("libexec/gnubin/man/man1/cat.1").is_file());
}

#[tokio::test]
async fn native_build_applies_patch_configures_and_installs_without_ruby() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("marker"), "before\n").unwrap();
    let patch = tmp.path().join("fix.patch");
    fs::write(
        &patch,
        "--- a/marker\n+++ b/marker\n@@ -1 +1 @@\n-before\n+after\n",
    )
    .unwrap();
    let configure = source.join("configure");
    // MOCKED: configure checks its patch/environment and emits a tiny Makefile.
    fs::write(
        &configure,
        r#"#!/bin/sh
set -eu
test "$(cat marker)" = after
test "$CC" = /usr/bin/clang
test "$CFLAGS" = -O2
test "$CONFIG_SITE" = /dev/null
prefix=
for arg do
  case "$arg" in
    --prefix=*) prefix=${arg#--prefix=} ;;
  esac
done
test -n "$prefix"
printf "install:\n\tmkdir -p '%s/bin' '%s/share/man/man1'\n\ttouch '%s/bin/gcat' '%s/share/man/man1/gcat.1'\n" "$prefix" "$prefix" "$prefix" "$prefix" > Makefile
"#,
    )
    .unwrap();
    fs::set_permissions(&configure, fs::Permissions::from_mode(0o755)).unwrap();
    let deps = fake_dependencies(tmp.path());
    let plan = plan(&tmp.path().join("keg with spaces"));
    build(&plan, &source, &patch, &deps).await.unwrap();
    create_compatibility_links(&plan.cellar_path).unwrap();
    assert_eq!(
        fs::read_to_string(source.join("marker")).unwrap(),
        "after\n"
    );
    assert!(plan.cellar_path.join("libexec/gnubin/cat").exists());
}

#[tokio::test]
async fn failing_command_is_not_ignored() {
    let tmp = tempfile::tempdir().unwrap();
    assert_matches!(
        run("/usr/bin/false", &[], tmp.path(), &[]).await,
        Err(Error::ExecutionError { message }) if message.contains("failed")
    );
}

/// Opt-in smoke test: uses explicitly supplied, checksum-verified source/patch
/// files and existing dependency kegs; writes only into a temporary directory.
#[tokio::test]
#[ignore = "requires coreutils source, patch, and gmp/openssl@4/texinfo kegs"]
async fn builds_real_coreutils() {
    use crate::checksum::verify_sha256_bytes;
    use crate::core::extraction::extract::extract_tarball;

    let input = |name| PathBuf::from(std::env::var_os(name).expect(name));
    let archive = input("UPKG_COREUTILS_TEST_ARCHIVE");
    let patch = input("UPKG_COREUTILS_TEST_PATCH");
    for (path, checksum) in [(&archive, SOURCE_SHA256), (&patch, PATCH_SHA256)] {
        verify_sha256_bytes(&fs::read(path).unwrap(), Some(checksum)).unwrap();
    }
    let deps = [
        ("gmp", "UPKG_COREUTILS_TEST_GMP"),
        ("openssl@4", "UPKG_COREUTILS_TEST_OPENSSL"),
        ("texinfo", "UPKG_COREUTILS_TEST_TEXINFO"),
    ]
    .map(|(name, env)| {
        (
            name.to_string(),
            DepInfo {
                cellar_path: input(env).to_str().unwrap().to_string(),
            },
        )
    })
    .into_iter()
    .collect();
    check_dependencies(&deps).unwrap();
    let formula = fs::read_to_string(input("UPKG_COREUTILS_TEST_FORMULA")).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let plan = plan(&tmp.path().join("Cellar/coreutils/9.12_1"));
    validate(&plan, &formula).unwrap();
    extract_tarball(&archive, tmp.path()).unwrap();
    build(&plan, &tmp.path().join("coreutils-9.12"), &patch, &deps)
        .await
        .unwrap();
    create_compatibility_links(&plan.cellar_path).unwrap();
    for command in ["gsha1sum", "gln", "gcat", "gprintenv"] {
        let output = Command::new(plan.cellar_path.join("bin").join(command))
            .arg("--version")
            .output()
            .await
            .unwrap();
        assert!(output.status.success(), "{command}");
        assert!(String::from_utf8_lossy(&output.stdout).contains("9.12"));
    }
    for command in ["printenv", "env"] {
        let output = Command::new(plan.cellar_path.join("libexec/gnubin").join(command))
            .env_clear()
            .env("UPKG_PATCH_TEST", "a b")
            .output()
            .await
            .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"UPKG_PATCH_TEST=a b\n");
    }
}
