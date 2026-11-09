//! A deliberately bounded native recipe, not an interpreter for arbitrary Ruby.
//! Updating the source, patch, or formula logic requires reviewing this recipe.
//!
//! Homebrew stopped bottling bash for macOS 14, so without this recipe upkg
//! cannot install it there at all.
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::core::storage::fs_util::StoreContext;
use crate::types::{BuildPlan, Error};

use super::executor::DepInfo;
use super::recipe;
use super::source::download_verified_source;

const FORMULA: &str = include_str!("bash/formula.rb");
const DEPENDENCY_FILES: &[(&str, &str)] = &[
    ("ncurses", "include/curses.h"),
    ("readline", "include/readline/readline.h"),
    ("gettext", "include/libintl.h"),
];
const RECIPE: &str = "bash";
const VERSION: &str = "5.3.20";
const SOURCE_SHA256: &str = "0d5cd86965f869a26cf64f4b71be7b96f90a3ba8b3d74e27e8e9d9d5550f31ba";
const PATCH_URL_PREFIX: &str = "https://ftpmirror.gnu.org/bash/bash-5.3-patches/bash53-";
/// The formula's `patch_checksum_pairs`, applied in order with `-p0`.
const PATCHES: &[(&str, &str)] = &[
    (
        "001",
        "1f608434364af86b9b45c8b0ea3fb3b165fb830d27697e6cdfc7ac17dee3287f",
    ),
    (
        "002",
        "e385548a00130765ec7938a56fbdca52447ab41fabc95a25f19ade527e282001",
    ),
    (
        "003",
        "f245d9c7dc3f5a20d84b53d249334747940936f09dc97e1dcb89fc3ab37d60ed",
    ),
    (
        "004",
        "9591d245045529f32f0812f94180b9d9ce9023f5a765c039b852e5dfc99747d0",
    ),
    (
        "005",
        "cca1ef52dbbf433bc98e33269b64b2c814028efe2538be1e2c9a377da90bc99d",
    ),
    (
        "006",
        "29119addefed8eff91ae37fd51822c31780ee30d4a28376e96002706c995ff10",
    ),
    (
        "007",
        "c0976bbfffa1453c7cfdd62058f206a318568ff2d690f5d4fa048793fa3eb299",
    ),
    (
        "008",
        "097cd723cbfb8907674ac32214063a3fd85282657ec5b4e544d2c0f719653fb4",
    ),
    (
        "009",
        "eee30fe78a4b0cb2fe20e010e00308899cfc613e0774ebb3c8557a1552f24f8c",
    ),
    (
        "010",
        "cf76f1cce2ea300c18bff9f002d21f280cc931acd17c28518110b93fe6e72569",
    ),
    (
        "011",
        "0298df8f5ea2a31d3be43ed7d269c5b3c7c342dd5b570bea7f64d66dcbbe7531",
    ),
    (
        "012",
        "d71379b39bebaedaf123414414e77fb458a0a43b9ad3116594c6df7ca6754573",
    ),
    (
        "013",
        "042f9cda967e24bf4211944697441e93d06ff42b4b998629a98a1b249279f200",
    ),
    (
        "014",
        "bd4360b401d38507e358783dcad8536a99c6789f0d3a5bd0cfb8c4a34144696c",
    ),
    (
        "015",
        "55b79ceee2fc27f6767eed697e939a7eb2fe2a28c01556bd75f18d581014f46e",
    ),
    (
        "016",
        "9ea29b266b7d24cb34d0ff3f1c4631e4d527bfe2d1ef15d17cdb924bf31ef767",
    ),
    (
        "017",
        "443b927b45c1558ca72052410f8b8f6e5152b617ed707061a2781d4375b0d1c3",
    ),
    (
        "018",
        "ae715d76c50341d7d7095e9a8d2eeed1ca9546152c2ac7289206f90cf30ac697",
    ),
    (
        "019",
        "a25c581e4d0057dea3833918438a930e2e86ee4c6dc17fe15267b7f04cbc4e3d",
    ),
    (
        "020",
        "df217ed3a9122aa2286d9b67bbe348661b6a9db262b580c29150dae55d532896",
    ),
];
const DEPENDENCIES: &[&str] = &["ncurses", "readline", "gettext"];
/// Stock bash loadable paths the formula keeps after `HOMEBREW_PREFIX/lib/bash`.
const STOCK_LOADABLE_PATHS: &[&str] = &[
    "/usr/local/lib/bash",
    "/usr/lib/bash",
    "/opt/local/lib/bash",
    "/usr/pkg/lib/bash",
    "/opt/pkg/lib/bash",
    ".",
];

pub(super) fn validate(plan: &BuildPlan, source: &str) -> Result<(), Error> {
    recipe::validate_reviewed(plan, RECIPE, VERSION, SOURCE_SHA256, FORMULA, source)
}

pub(super) fn check_dependencies(deps: &HashMap<String, DepInfo>) -> Result<(), Error> {
    recipe::check_dependency_files(RECIPE, deps, DEPENDENCY_FILES)
}

pub(super) async fn check_linkage(deps: &HashMap<String, DepInfo>) -> Result<(), Error> {
    recipe::check_linkage(
        RECIPE,
        deps,
        &[
            ("ncurses", "lib/libncursesw.dylib"),
            ("readline", "lib/libreadline.dylib"),
            ("gettext", "lib/libintl.dylib"),
        ],
    )
    .await
}

pub(super) async fn execute(
    plan: &BuildPlan,
    source_root: &Path,
    work_dir: &Path,
    deps: &HashMap<String, DepInfo>,
) -> Result<(), Error> {
    let mut patches = Vec::with_capacity(PATCHES.len());
    for (number, sha256) in PATCHES {
        let path = work_dir.join(format!("bash53-{number}"));
        download_verified_source(&format!("{PATCH_URL_PREFIX}{number}"), Some(sha256), &path)
            .await?;
        patches.push(path);
    }
    build(plan, source_root, &patches, deps, strchrnul_broken()).await?;
    finish_install(&plan.cellar_path)
}

async fn build(
    plan: &BuildPlan,
    source_root: &Path,
    patches: &[PathBuf],
    deps: &HashMap<String, DepInfo>,
    strchrnul_broken: bool,
) -> Result<(), Error> {
    let env = build_environment(&plan.prefix, deps, strchrnul_broken)?;
    for patch in patches {
        recipe::run(
            RECIPE,
            "/usr/bin/patch",
            &[
                OsString::from("-p0"),
                OsString::from("--batch"),
                OsString::from("--forward"),
                OsString::from("-i"),
                patch.as_os_str().to_owned(),
            ],
            source_root,
            &env,
        )
        .await?;
    }
    let mut prefix_arg = OsString::from("--prefix=");
    prefix_arg.push(&plan.cellar_path);
    recipe::run(
        RECIPE,
        "./configure",
        &[
            prefix_arg,
            OsString::from("--with-curses"),
            OsString::from("--with-installed-readline"),
        ],
        source_root,
        &env,
    )
    .await?;
    let jobs = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    recipe::run(
        RECIPE,
        "/usr/bin/make",
        &[
            OsString::from(format!("-j{jobs}")),
            OsString::from("install"),
        ],
        source_root,
        &env,
    )
    .await
}

/// The formula's `bash_loadables_path`: `HOMEBREW_PREFIX/lib/bash` first, then
/// the stock locations, deduplicated.
fn loadables_path(prefix: &Path) -> String {
    let mut paths = vec![prefix.join("lib/bash").to_string_lossy().into_owned()];
    for stock in STOCK_LOADABLE_PATHS {
        if !paths.iter().any(|path| path == stock) {
            paths.push((*stock).to_string());
        }
    }
    paths.join(":")
}

fn build_environment(
    prefix: &Path,
    deps: &HashMap<String, DepInfo>,
    strchrnul_broken: bool,
) -> Result<Vec<(&'static str, OsString)>, Error> {
    let mut paths = Vec::new();
    let mut cppflags = Vec::new();
    let mut ldflags = Vec::new();
    for name in DEPENDENCIES {
        let dep = recipe::dependency_path(RECIPE, deps, name)?;
        paths.push(dep.join("bin"));
        let (include, lib) = recipe::include_and_lib_flags(dep)?;
        cppflags.push(include);
        ldflags.push(lib);
    }
    paths.extend(["/usr/bin", "/bin", "/usr/sbin", "/sbin"].map(PathBuf::from));
    let path = std::env::join_paths(paths).map_err(|e| Error::ExecutionError {
        message: format!("invalid bash build PATH: {e}"),
    })?;
    // Same flags the formula appends: macOS's /bin/bash sources ~/.bashrc for
    // sshd commands, and the loadables path is baked in as a C string.
    let cflags = format!(
        "-O2 -DSSH_SOURCE_BASHRC -DDEFAULT_LOADABLE_BUILTINS_PATH='\"{}\"'",
        loadables_path(prefix)
    );
    let mut env = vec![
        ("PATH", path),
        ("CC", "/usr/bin/clang".into()),
        ("CFLAGS", cflags.into()),
        ("CPPFLAGS", cppflags.join(" ").into()),
        ("LDFLAGS", ldflags.join(" ").into()),
        ("LC_ALL", "C".into()),
        ("CONFIG_SITE", "/dev/null".into()),
    ];
    if strchrnul_broken {
        // The formula's guard against crashes on macOS 15.0-15.4.
        env.push(("bash_cv_func_strchrnul_works", "no".into()));
    }
    Ok(env)
}

/// `MacOS.version <= :sequoia`. An unreadable version gets the safe answer:
/// bash's own strchrnul works everywhere.
fn strchrnul_broken() -> bool {
    std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok()
        .and_then(|output| macos_major(&String::from_utf8_lossy(&output.stdout)))
        .is_none_or(|major| major <= 15)
}

fn macos_major(version: &str) -> Option<u32> {
    version.trim().split('.').next()?.parse().ok()
}

/// The formula's post-`make install` moves: the loadables header into
/// `include/bash/builtins`, the loadables Makefiles into `share/bash`.
fn finish_install(keg: &Path) -> Result<(), Error> {
    let loadables = keg.join("lib/bash");
    let builtins = keg.join("include/bash/builtins");
    let pkgshare = keg.join("share/bash");
    std::fs::create_dir_all(&builtins)
        .store_context(|| "failed to create bash builtins include dir")?;
    std::fs::create_dir_all(&pkgshare).store_context(|| "failed to create bash pkgshare")?;
    move_file(
        &loadables.join("loadables.h"),
        &builtins.join("loadables.h"),
    )?;
    let entries = std::fs::read_dir(&loadables)
        .store_context(|| format!("failed to read '{}'", loadables.display()))?
        .collect::<Result<Vec<_>, _>>()
        .store_context(|| "failed to read bash loadables")?;
    for entry in entries {
        if entry.file_name().to_string_lossy().starts_with("Makefile") {
            move_file(&entry.path(), &pkgshare.join(entry.file_name()))?;
        }
    }
    Ok(())
}

fn move_file(from: &Path, to: &Path) -> Result<(), Error> {
    std::fs::rename(from, to)
        .store_context(|| format!("failed to move '{}' to '{}'", from.display(), to.display()))
}

#[cfg(test)]
#[path = "bash/tests.rs"]
mod tests;
