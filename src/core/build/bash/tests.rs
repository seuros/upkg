use super::*;
use crate::types::BuildSystem;
use std::assert_matches;
use std::fs;

fn plan(keg: &Path) -> BuildPlan {
    BuildPlan {
        formula_name: "bash".to_string(),
        version: VERSION.to_string(),
        source_url: "https://ftpmirror.gnu.org/bash/bash-5.3.tar.gz".to_string(),
        source_checksum: Some(SOURCE_SHA256.to_string()),
        ruby_source_path: None,
        build_dependencies: Vec::new(),
        runtime_dependencies: DEPENDENCIES.iter().map(ToString::to_string).collect(),
        detected_system: BuildSystem::Autoconf,
        prefix: keg.parent().unwrap().join("prefix"),
        cellar_path: keg.to_path_buf(),
    }
}

#[test]
fn accepts_reviewed_formula_and_bottle_rebuilds() {
    assert!(FORMULA.contains(SOURCE_SHA256));
    for (number, sha256) in PATCHES {
        assert!(
            FORMULA.contains(&format!("{number} {sha256}")),
            "patch {number}"
        );
    }
    // Homebrew versions bash as the release plus its upstream patch level
    assert_eq!(VERSION, format!("5.3.{}", PATCHES.len()));

    let plan = plan(Path::new("/tmp/bash"));
    validate(&plan, FORMULA).unwrap();
    let rebuilt = FORMULA.replace(
        "class Bash < Formula",
        "class Bash < Formula\n  bottle do\n    sha256 arm64_sonoma: \"new-bottle\"\n  end",
    );
    validate(&plan, &rebuilt).unwrap();
}

#[rstest::rstest]
#[case("--with-curses", "--without-curses")]
#[case("-DSSH_SOURCE_BASHRC", "-DSOMETHING_ELSE")]
#[case("type :cherry_pick", "type :other")]
#[case("\"/usr/lib/bash\",\n", "")]
#[case(
    "020 df217ed3a9122aa2286d9b67bbe348661b6a9db262b580c29150dae55d532896",
    "020 df217ed3a9122aa2286d9b67bbe348661b6a9db262b580c29150dae55d532896\n      021 0000000000000000000000000000000000000000000000000000000000000000"
)]
#[case("deny_network_access!", "deny_network_access!\n  patch :DATA")]
#[case(
    "  def install",
    "  def post_install\n    system \"unexpected\"\n  end\n  def install"
)]
fn rejects_changed_formula_logic(#[case] from: &str, #[case] to: &str) {
    let changed = FORMULA.replace(from, to);
    assert_ne!(changed, FORMULA);
    assert_matches!(
        validate(&plan(Path::new("/tmp/bash")), &changed),
        Err(Error::UnsupportedFormula { .. })
    );
}

#[test]
fn rejects_changed_source_or_version() {
    let mut plan = plan(Path::new("/tmp/bash"));
    plan.version = "5.3.21".to_string();
    assert!(validate(&plan, FORMULA).is_err());
    plan.version = VERSION.to_string();
    plan.source_checksum = None;
    assert!(validate(&plan, FORMULA).is_err());
    plan.source_checksum = Some(SOURCE_SHA256.to_string());
    plan.formula_name = "zsh".to_string();
    assert!(validate(&plan, FORMULA).is_err());
}

#[test]
fn loadables_path_puts_prefix_first_without_duplicates() {
    assert_eq!(
        loadables_path(Path::new("/opt/homebrew")),
        "/opt/homebrew/lib/bash:/usr/local/lib/bash:/usr/lib/bash:/opt/local/lib/bash:/usr/pkg/lib/bash:/opt/pkg/lib/bash:."
    );
    // Intel prefix: the formula's `.uniq` drops the repeated stock path
    assert_eq!(
        loadables_path(Path::new("/usr/local")),
        "/usr/local/lib/bash:/usr/lib/bash:/opt/local/lib/bash:/usr/pkg/lib/bash:/opt/pkg/lib/bash:."
    );
}

#[test]
fn parses_macos_major_version() {
    for (output, expected) in [
        ("14.8.9\n", Some(14)),
        ("26.0", Some(26)),
        ("15", Some(15)),
        ("", None),
        ("garbage", None),
    ] {
        assert_eq!(macos_major(output), expected, "output: {output:?}");
    }
}

fn fake_dependencies(root: &Path) -> HashMap<String, DepInfo> {
    let mut deps = HashMap::new();
    for (name, file) in [
        ("ncurses", "include/curses.h"),
        ("readline", "include/readline/readline.h"),
        ("gettext", "include/libintl.h"),
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
fn environment_carries_formula_flags_and_quoted_kegs() {
    let tmp = tempfile::tempdir().unwrap();
    let deps = fake_dependencies(&tmp.path().join("dependencies with spaces"));
    check_dependencies(&deps).unwrap();

    let env: HashMap<_, _> = build_environment(Path::new("/opt/homebrew"), &deps, true)
        .unwrap()
        .into_iter()
        .collect();
    let paths: Vec<_> = std::env::split_paths(&env["PATH"]).collect();
    assert_eq!(
        paths[0],
        Path::new(&deps["ncurses"].cellar_path).join("bin")
    );
    let flags = shlex::split(env["CPPFLAGS"].to_str().unwrap()).unwrap();
    assert_eq!(
        flags[1],
        format!(
            "-I{}",
            Path::new(&deps["readline"].cellar_path)
                .join("include")
                .display()
        )
    );
    let cflags = env["CFLAGS"].to_str().unwrap();
    assert!(cflags.contains("-DSSH_SOURCE_BASHRC"));
    assert!(cflags.contains("-DDEFAULT_LOADABLE_BUILTINS_PATH='\"/opt/homebrew/lib/bash:"));
    assert_eq!(env["bash_cv_func_strchrnul_works"], "no");

    let newer: HashMap<_, _> = build_environment(Path::new("/opt/homebrew"), &deps, false)
        .unwrap()
        .into_iter()
        .collect();
    assert!(!newer.contains_key("bash_cv_func_strchrnul_works"));

    fs::remove_file(Path::new(&deps["gettext"].cellar_path).join("include/libintl.h")).unwrap();
    assert!(check_dependencies(&deps).is_err());
    assert!(check_dependencies(&HashMap::new()).is_err());
}

#[tokio::test]
async fn native_build_applies_patches_in_order_configures_and_installs() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("marker"), "base\n").unwrap();
    // Upstream bash patches are -p0: paths carry no a/ b/ prefix
    let patches =
        [("first", "base", "middle"), ("second", "middle", "patched")].map(|(name, from, to)| {
            let path = tmp.path().join(name);
            fs::write(
                &path,
                format!("--- marker\n+++ marker\n@@ -1 +1 @@\n-{from}\n+{to}\n"),
            )
            .unwrap();
            path
        });
    let configure = source.join("configure");
    // MOCKED: configure checks patches, flags and environment, then emits a
    // Makefile that installs the files the formula moves afterwards.
    fs::write(
        &configure,
        r#"#!/bin/sh
set -eu
test "$(cat marker)" = patched
test "$CC" = /usr/bin/clang
test "$CONFIG_SITE" = /dev/null
test "$bash_cv_func_strchrnul_works" = no
case "$CFLAGS" in *-DSSH_SOURCE_BASHRC*) ;; *) exit 1 ;; esac
prefix= curses= readline=
for arg do
  case "$arg" in
    --prefix=*) prefix=${arg#--prefix=} ;;
    --with-curses) curses=1 ;;
    --with-installed-readline) readline=1 ;;
  esac
done
test -n "$prefix" && test -n "$curses" && test -n "$readline"
printf "install:\n\tmkdir -p '%s/bin' '%s/lib/bash'\n\ttouch '%s/bin/bash' '%s/lib/bash/loadables.h' '%s/lib/bash/Makefile.inc' '%s/lib/bash/Makefile.sample' '%s/lib/bash/csv'\n" "$prefix" "$prefix" "$prefix" "$prefix" "$prefix" "$prefix" "$prefix" > Makefile
"#,
    )
    .unwrap();
    fs::set_permissions(&configure, fs::Permissions::from_mode(0o755)).unwrap();

    let deps = fake_dependencies(tmp.path());
    let plan = plan(&tmp.path().join("keg with spaces"));
    build(&plan, &source, &patches, &deps, true).await.unwrap();
    finish_install(&plan.cellar_path).unwrap();

    let keg = &plan.cellar_path;
    assert!(keg.join("bin/bash").is_file());
    assert!(keg.join("include/bash/builtins/loadables.h").is_file());
    assert!(keg.join("share/bash/Makefile.inc").is_file());
    assert!(keg.join("share/bash/Makefile.sample").is_file());
    assert!(!keg.join("lib/bash/loadables.h").exists());
    assert!(!keg.join("lib/bash/Makefile.inc").exists());
    // Loadable builtins stay where DEFAULT_LOADABLE_BUILTINS_PATH looks
    assert!(keg.join("lib/bash/csv").is_file());
}
