//! Plumbing shared by the bounded native recipes (coreutils, bash): reviewed
//! formula snapshots, dependency kegs, and network-sandboxed build steps.
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;
use std::process::Stdio;

use tokio::process::Command;
use tree_sitter::Node;

use crate::types::{BuildPlan, Error};

use super::executor::DepInfo;
use super::formula_parser::parse_ruby;

/// Sources and patches are downloaded and verified before any build step, so
/// the steps themselves never need the network.
const NETWORK_SANDBOX: &str = "(version 1) (allow default) (deny network*)";

/// Refuse anything but the reviewed formula: same name, version, source
/// checksum, and syntax tree (bottle rebuilds aside).
pub(super) fn validate_reviewed(
    plan: &BuildPlan,
    recipe: &str,
    version: &str,
    source_sha256: &str,
    reviewed: &str,
    source: &str,
) -> Result<(), Error> {
    if plan.formula_name != recipe
        || plan.version != version
        || plan.source_checksum.as_deref() != Some(source_sha256)
        || formula_tokens(recipe, source)? != formula_tokens(recipe, reviewed)?
    {
        return Err(Error::UnsupportedFormula {
            name: plan.formula_name.clone(),
            reason: format!(
                "native {recipe} recipe supports the reviewed Homebrew {version} formula only; source, patches, or formula logic have changed"
            ),
        });
    }
    Ok(())
}

pub(super) fn formula_tokens(recipe: &str, source: &str) -> Result<Vec<(String, String)>, Error> {
    let tree = parse_ruby(source)?;
    if tree.root_node().has_error() {
        return Err(Error::UnsupportedFormula {
            name: recipe.to_string(),
            reason: format!("invalid Ruby syntax in {recipe} formula"),
        });
    }
    let mut tokens = Vec::new();
    collect_tokens(tree.root_node(), source.as_bytes(), &mut tokens);
    Ok(tokens)
}

fn collect_tokens(node: Node<'_>, source: &[u8], tokens: &mut Vec<(String, String)>) {
    // Bottle rebuilds do not affect source builds. Everything else, including
    // helpers, patches, and additional class-level DSL, must match the snapshot.
    if node.kind() == "comment"
        || (node.kind() == "call"
            && node.parent().is_some_and(|parent| {
                parent.kind() == "body_statement"
                    && parent.parent().is_some_and(|owner| owner.kind() == "class")
            })
            && node
                .child_by_field_name("method")
                .is_some_and(|method| method.utf8_text(source) == Ok("bottle"))
            && node.child_by_field_name("receiver").is_none())
    {
        return;
    }
    if node.child_count() == 0 {
        tokens.push((
            node.kind().to_string(),
            node.utf8_text(source).unwrap_or_default().to_string(),
        ));
    } else {
        tokens.push((node.kind().to_string(), String::new()));
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            collect_tokens(child, source, tokens);
        }
        tokens.push(("/".to_string(), node.kind().to_string()));
    }
}

pub(super) fn dependency_path<'a>(
    recipe: &str,
    deps: &'a HashMap<String, DepInfo>,
    name: &str,
) -> Result<&'a Path, Error> {
    deps.get(name)
        .map(|dep| Path::new(&dep.cellar_path))
        .ok_or_else(|| Error::ExecutionError {
            message: format!("{recipe} source build requires installed dependency '{name}'"),
        })
}

/// Each `(dependency, relative path)` must exist in the dependency's keg.
pub(super) fn check_dependency_files(
    recipe: &str,
    deps: &HashMap<String, DepInfo>,
    required: &[(&str, &str)],
) -> Result<(), Error> {
    for (name, file) in required {
        let path = dependency_path(recipe, deps, name)?;
        if !path.join(file).is_file() {
            return Err(Error::ExecutionError {
                message: format!(
                    "{recipe} build dependency '{name}' is missing '{file}' in '{}'",
                    path.display()
                ),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn fake_dependencies(
    required: &[(&str, &str)],
    root: &Path,
) -> HashMap<String, DepInfo> {
    required
        .iter()
        .map(|(name, file)| {
            let keg = root.join(name);
            let path = keg.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "").unwrap();
            (
                (*name).to_string(),
                DepInfo {
                    cellar_path: keg.to_str().unwrap().to_string(),
                },
            )
        })
        .collect()
}

/// Each `(dependency, dylib)` must carry an absolute install name that exists,
/// or the built binary would fail to load it.
pub(super) async fn check_linkage(
    recipe: &str,
    deps: &HashMap<String, DepInfo>,
    libraries: &[(&str, &str)],
) -> Result<(), Error> {
    for (name, library) in libraries {
        let path = dependency_path(recipe, deps, name)?.join(library);
        let output = Command::new("/usr/bin/otool")
            .arg("-D")
            .arg(&path)
            .output()
            .await
            .map_err(|e| Error::ExecutionError {
                message: format!("failed to inspect {recipe} dependency '{name}': {e}"),
            })?;
        if !output.status.success() {
            return Err(Error::ExecutionError {
                message: format!(
                    "cannot inspect {recipe} dependency '{}': {}; install Apple's Command Line Tools and verify the dependency keg",
                    name,
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            });
        }
        let text = String::from_utf8_lossy(&output.stdout);
        let id = text.lines().nth(1).unwrap_or_default().trim();
        validate_library_id(recipe, name, id)?;
    }
    Ok(())
}

pub(super) fn validate_library_id(recipe: &str, name: &str, id: &str) -> Result<(), Error> {
    if !id.starts_with('/') || !Path::new(id).is_file() {
        return Err(Error::ExecutionError {
            message: format!(
                "{recipe} dependency '{name}' has an unavailable dylib install name '{id}'; reinstall or repair this dependency before building {recipe}"
            ),
        });
    }
    Ok(())
}

/// `-I<keg>/include` and `-L<keg>/lib`, shell-quoted for CPPFLAGS/LDFLAGS.
pub(super) fn include_and_lib_flags(keg: &Path) -> Result<(String, String), Error> {
    let flag = |option: &str, subdir: &str| {
        let path = keg.join(subdir);
        let text = path.to_string_lossy();
        shlex::try_quote(&text)
            .map(|quoted| format!("{option}{quoted}"))
            .map_err(|e| Error::ExecutionError {
                message: format!("invalid build dependency path: {e}"),
            })
    };
    Ok((flag("-I", "include")?, flag("-L", "lib")?))
}

/// Run one build step with only `env`, no stdin, and no network.
pub(super) async fn run(
    recipe: &str,
    program: &str,
    args: &[OsString],
    source_root: &Path,
    env: &[(&str, OsString)],
) -> Result<(), Error> {
    eprintln!("    Building {recipe}: {program}");
    let status = Command::new("/usr/bin/sandbox-exec")
        .args(["-p", NETWORK_SANDBOX, program])
        .args(args)
        .current_dir(source_root)
        .env_clear()
        .envs(env.iter().map(|(key, value)| (*key, value)))
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .status()
        .await
        .map_err(|e| Error::ExecutionError {
            message: format!("failed to run {recipe} build step '{program}': {e}"),
        })?;
    if !status.success() {
        return Err(Error::ExecutionError {
            message: format!(
                "{recipe} build step '{program}' failed ({status}); see build output in '{}'",
                source_root.display()
            ),
        });
    }
    Ok(())
}
