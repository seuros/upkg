//! A deliberately bounded native recipe, not an interpreter for arbitrary Ruby.
//! Updating the source, patch, or formula logic requires reviewing this recipe.
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::core::storage::fs_util::StoreContext;
use crate::types::{BuildPlan, Error};

use super::executor::DepInfo;
use super::recipe;
use super::source::download_verified_source;

const FORMULA: &str = include_str!("coreutils/formula.rb");
const SOURCE_SHA256: &str = "a480198559733e9b3da999e90543ac6f888a2caa544d8d664c5a1f17e528e210";
const PATCH_URL: &str = "https://github.com/coreutils/coreutils/commit/782a1e5bc2090212273bb731dceee2cc2a071e54.patch?full_index=1";
const PATCH_SHA256: &str = "d93cf338341d9418522a637e3c99c4211a25a967d0cffd2f036fd18871f15e35";
const DEPENDENCIES: &[&str] = &["gmp", "openssl@4", "texinfo"];
const RECIPE: &str = "coreutils";

pub(super) fn validate(plan: &BuildPlan, source: &str) -> Result<(), Error> {
    recipe::validate_reviewed(plan, RECIPE, "9.12_1", SOURCE_SHA256, FORMULA, source)
}

#[cfg(test)]
fn formula_tokens(source: &str) -> Result<Vec<(String, String)>, Error> {
    recipe::formula_tokens(RECIPE, source)
}

pub(super) fn check_dependencies(deps: &HashMap<String, DepInfo>) -> Result<(), Error> {
    recipe::check_dependency_files(
        RECIPE,
        deps,
        &[
            ("gmp", "include/gmp.h"),
            ("openssl@4", "include/openssl/ssl.h"),
            ("texinfo", "bin/makeinfo"),
        ],
    )
}

fn dependency_path<'a>(deps: &'a HashMap<String, DepInfo>, name: &str) -> Result<&'a Path, Error> {
    recipe::dependency_path(RECIPE, deps, name)
}

pub(super) async fn check_linkage(deps: &HashMap<String, DepInfo>) -> Result<(), Error> {
    recipe::check_linkage(
        RECIPE,
        deps,
        &[
            ("gmp", "lib/libgmp.dylib"),
            ("openssl@4", "lib/libcrypto.dylib"),
        ],
    )
    .await
}

#[cfg(test)]
fn validate_library_id(name: &str, id: &str) -> Result<(), Error> {
    recipe::validate_library_id(RECIPE, name, id)
}

pub(super) async fn execute(
    plan: &BuildPlan,
    source_root: &Path,
    work_dir: &Path,
    deps: &HashMap<String, DepInfo>,
) -> Result<(), Error> {
    let patch_path = work_dir.join("coreutils.patch");
    download_verified_source(PATCH_URL, Some(PATCH_SHA256), &patch_path).await?;
    build(plan, source_root, &patch_path, deps).await?;
    create_compatibility_links(&plan.cellar_path)
}

async fn build(
    plan: &BuildPlan,
    source_root: &Path,
    patch_path: &Path,
    deps: &HashMap<String, DepInfo>,
) -> Result<(), Error> {
    let env = build_environment(deps)?;
    run(
        "/usr/bin/patch",
        &[
            OsString::from("-p1"),
            OsString::from("--batch"),
            OsString::from("--forward"),
            OsString::from("-i"),
            patch_path.as_os_str().to_owned(),
        ],
        source_root,
        &env,
    )
    .await?;
    let mut prefix_arg = OsString::from("--prefix=");
    prefix_arg.push(&plan.cellar_path);
    let mut libdir_arg = OsString::from("--libdir=");
    libdir_arg.push(plan.cellar_path.join("lib"));
    run(
        "./configure",
        &[
            OsString::from("--program-prefix=g"),
            OsString::from("--with-libgmp"),
            OsString::from("--without-selinux"),
            OsString::from("--disable-dependency-tracking"),
            prefix_arg,
            libdir_arg,
        ],
        source_root,
        &env,
    )
    .await?;
    let jobs = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    run(
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

fn build_environment(
    deps: &HashMap<String, DepInfo>,
) -> Result<Vec<(&'static str, OsString)>, Error> {
    let mut paths = Vec::new();
    let mut cppflags = Vec::new();
    let mut ldflags = Vec::new();
    for name in DEPENDENCIES {
        let dep = dependency_path(deps, name)?;
        paths.push(dep.join("bin"));
        if *name == "texinfo" {
            continue;
        }
        let (include, lib) = recipe::include_and_lib_flags(dep)?;
        cppflags.push(include);
        ldflags.push(lib);
    }
    paths.extend(["/usr/bin", "/bin", "/usr/sbin", "/sbin"].map(PathBuf::from));
    let path = std::env::join_paths(paths).map_err(|e| Error::ExecutionError {
        message: format!("invalid coreutils build PATH: {e}"),
    })?;
    Ok(vec![
        ("PATH", path),
        ("CC", "/usr/bin/clang".into()),
        ("CXX", "/usr/bin/clang++".into()),
        // Never use -march=native: the formula requests runtime CPU detection.
        ("CFLAGS", "-O2".into()),
        ("CXXFLAGS", "-O2".into()),
        ("CPPFLAGS", cppflags.join(" ").into()),
        ("LDFLAGS", ldflags.join(" ").into()),
        ("LC_ALL", "C".into()),
        ("CONFIG_SITE", "/dev/null".into()),
    ])
}

async fn run(
    program: &str,
    args: &[OsString],
    source_root: &Path,
    env: &[(&str, OsString)],
) -> Result<(), Error> {
    recipe::run(RECIPE, program, args, source_root, env).await
}

fn create_compatibility_links(keg: &Path) -> Result<(), Error> {
    let bin = keg.join("bin");
    let man1 = keg.join("share/man/man1");
    let gnubin = keg.join("libexec/gnubin");
    let gnuman = keg.join("libexec/gnuman/man1");
    std::fs::create_dir_all(&gnubin).store_context(|| "failed to create coreutils gnubin")?;
    std::fs::create_dir_all(&gnuman).store_context(|| "failed to create coreutils gnuman")?;
    for (directory, destination, relative) in [
        (&bin, &gnubin, "../../bin"),
        (&man1, &gnuman, "../../../share/man/man1"),
    ] {
        // Snapshot before adding unprefixed links into the same directory.
        let entries = std::fs::read_dir(directory)
            .store_context(|| {
                format!(
                    "failed to read coreutils directory '{}'",
                    directory.display()
                )
            })?
            .collect::<Result<Vec<_>, _>>()
            .store_context(|| "failed to read coreutils files")?;
        for entry in entries {
            if entry.path().is_dir() {
                continue;
            }
            let filename = entry.file_name();
            let filename = filename.to_str().ok_or_else(|| Error::ExecutionError {
                message: "non-UTF-8 filename in coreutils install".to_string(),
            })?;
            let Some(command) = filename.strip_prefix('g') else {
                continue;
            };
            symlink(
                &Path::new(relative).join(filename),
                &destination.join(command),
            )?;
            if directory == &bin
                && !["dir", "dircolors", "vdir"].contains(&command)
                && !system_command_exists(command)
            {
                symlink(Path::new(filename), &bin.join(command))?;
                let man = format!("{filename}.1");
                symlink(Path::new(&man), &man1.join(format!("{command}.1")))?;
            }
        }
    }
    symlink(Path::new("../gnuman"), &gnubin.join("man"))
}

fn system_command_exists(command: &str) -> bool {
    // Check all system locations, not the user's PATH (which may already contain
    // this keg). Never replace a macOS command with an unprefixed GNU command.
    ["/usr/bin", "/bin", "/usr/sbin", "/sbin"]
        .iter()
        .any(|directory| Path::new(directory).join(command).exists())
}

fn symlink(target: &Path, link: &Path) -> Result<(), Error> {
    std::os::unix::fs::symlink(target, link)
        .store_context(|| format!("failed to create coreutils link '{}'", link.display()))
}

#[cfg(test)]
#[path = "coreutils/tests.rs"]
mod tests;
