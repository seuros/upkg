use crate::package_ref::cask_name;
use crate::types::Error;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaskBinary {
    pub source: String,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaskApp {
    pub source: String,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaskPkg {
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaskUninstall {
    pub pkgutil: Vec<String>,
    pub delete: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaskLinkedArtifactKind {
    Manpage,
    BashCompletion,
    FishCompletion,
    ZshCompletion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaskLinkedArtifact {
    pub kind: CaskLinkedArtifactKind,
    pub source: String,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaskPostflightSymlink {
    pub source: String,
    pub target: String,
    pub skip_if_exists: bool,
    pub uninstall: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCask {
    pub install_name: String,
    pub token: String,
    pub version: String,
    pub url: String,
    pub sha256: String,
    pub binaries: Vec<CaskBinary>,
    pub apps: Vec<CaskApp>,
    pub pkgs: Vec<CaskPkg>,
    pub uninstall: CaskUninstall,
    pub linked_artifacts: Vec<CaskLinkedArtifact>,
    pub postflight_symlinks: Vec<CaskPostflightSymlink>,
}

pub fn resolve_cask(token: &str, cask: &Value) -> Result<ResolvedCask, Error> {
    let token = cask.get("token").and_then(Value::as_str).unwrap_or(token);
    let mut url = required_string(cask, "url")?;
    let mut sha256 = required_string(cask, "sha256")?;
    let version = required_string(cask, "version")?;

    if let Some(variation) = select_platform_variation(cask) {
        for (key, value) in [("url", &mut url), ("sha256", &mut sha256)] {
            if let Some(overridden) = variation.get(key).and_then(Value::as_str) {
                *value = overridden.to_string();
            }
        }
    }

    if sha256 == "no_check" {
        return Err(Error::InvalidArgument {
            message: format!("cask '{token}' uses an unsupported checksum mode: no_check"),
        });
    }

    let binaries = parse_binary_artifacts(cask)?;
    let apps = parse_app_artifacts(cask)?;
    let pkgs = parse_pkg_artifacts(cask)?;
    let uninstall = parse_uninstall_artifacts(cask)?;
    let linked_artifacts = parse_linked_artifacts(cask)?;
    let postflight_symlinks = parse_postflight_symlinks(cask)?;
    if binaries.is_empty() && apps.is_empty() && pkgs.is_empty() {
        return Err(Error::InvalidArgument {
            message: format!(
                "cask '{token}' does not expose supported app, pkg, or binary artifacts"
            ),
        });
    }

    Ok(ResolvedCask {
        install_name: cask_name(token),
        token: token.to_string(),
        version,
        url,
        sha256,
        binaries,
        apps,
        pkgs,
        uninstall,
        linked_artifacts,
        postflight_symlinks,
    })
}

fn required_string(value: &Value, field: &str) -> Result<String, Error> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| Error::InvalidArgument {
            message: format!("failed to parse cask JSON: missing string field '{field}'"),
        })
}

fn select_platform_variation(cask: &Value) -> Option<&Value> {
    let variations = cask.get("variations")?;
    variations.get(current_macos_variation_key())
}

fn current_macos_variation_key() -> String {
    crate::types::formula::bottle::current_platform_bottle_tag().unwrap_or_else(|| {
        #[cfg(target_arch = "aarch64")]
        {
            "arm64_sequoia".to_string()
        }
        #[cfg(target_arch = "x86_64")]
        {
            "sequoia".to_string()
        }
    })
}

fn parse_binary_artifacts(cask: &Value) -> Result<Vec<CaskBinary>, Error> {
    let mut binaries = Vec::new();

    for artifact in artifacts(cask)? {
        let Some(entries) = artifact.get("binary").and_then(Value::as_array) else {
            continue;
        };

        let sibling_target = artifact_target(artifact);
        if is_flat_artifact_entry(entries) {
            let (source, target) =
                parse_binary_entry(&Value::Array(entries.clone()), sibling_target)?;
            binaries.push(CaskBinary { source, target });
        } else {
            for entry in entries {
                let fallback_target = (entries.len() == 1).then_some(sibling_target).flatten();
                let (source, target) = parse_binary_entry(entry, fallback_target)?;
                binaries.push(CaskBinary { source, target });
            }
        }
    }

    Ok(binaries)
}

fn parse_app_artifacts(cask: &Value) -> Result<Vec<CaskApp>, Error> {
    let mut apps = Vec::new();

    for artifact in artifacts(cask)? {
        let Some(entries) = artifact.get("app").and_then(Value::as_array) else {
            continue;
        };

        let sibling_target = artifact_target(artifact);
        if is_flat_artifact_entry(entries) {
            let (source, target) = parse_app_entry(&Value::Array(entries.clone()), sibling_target)?;
            apps.push(CaskApp { source, target });
        } else {
            for entry in entries {
                let fallback_target = (entries.len() == 1).then_some(sibling_target).flatten();
                let (source, target) = parse_app_entry(entry, fallback_target)?;
                apps.push(CaskApp { source, target });
            }
        }
    }

    Ok(apps)
}

fn parse_pkg_artifacts(cask: &Value) -> Result<Vec<CaskPkg>, Error> {
    let mut pkgs = Vec::new();

    for artifact in artifacts(cask)? {
        let Some(entries) = artifact.get("pkg").and_then(Value::as_array) else {
            continue;
        };

        if let Some(source) = entries.first().and_then(Value::as_str) {
            pkgs.push(CaskPkg {
                source: source.to_string(),
            });
            continue;
        }

        for entry in entries {
            let Some(source) = entry.as_str() else {
                continue;
            };
            pkgs.push(CaskPkg {
                source: source.to_string(),
            });
        }
    }

    Ok(pkgs)
}

fn parse_uninstall_artifacts(cask: &Value) -> Result<CaskUninstall, Error> {
    let mut uninstall = CaskUninstall {
        pkgutil: Vec::new(),
        delete: Vec::new(),
    };

    for artifact in artifacts(cask)? {
        let Some(entries) = artifact.get("uninstall").and_then(Value::as_array) else {
            continue;
        };

        for entry in entries {
            let Some(obj) = entry.as_object() else {
                continue;
            };
            extend_string_or_strings(obj.get("pkgutil"), &mut uninstall.pkgutil);
            extend_string_or_strings(obj.get("delete"), &mut uninstall.delete);
        }
    }

    Ok(uninstall)
}

fn extend_string_or_strings(value: Option<&Value>, out: &mut Vec<String>) {
    match value {
        Some(Value::String(s)) => out.push(s.clone()),
        Some(Value::Array(values)) => {
            out.extend(values.iter().filter_map(Value::as_str).map(str::to_string))
        }
        _ => {}
    }
}

fn parse_linked_artifacts(cask: &Value) -> Result<Vec<CaskLinkedArtifact>, Error> {
    let mut linked_artifacts = Vec::new();

    for artifact in artifacts(cask)? {
        linked_artifacts.extend(parse_linked_artifact_entries(
            artifact,
            "manpage",
            CaskLinkedArtifactKind::Manpage,
            manpage_target,
        )?);
        linked_artifacts.extend(parse_linked_artifact_entries(
            artifact,
            "bash_completion",
            CaskLinkedArtifactKind::BashCompletion,
            bash_completion_target,
        )?);
        linked_artifacts.extend(parse_linked_artifact_entries(
            artifact,
            "fish_completion",
            CaskLinkedArtifactKind::FishCompletion,
            fish_completion_target,
        )?);
        linked_artifacts.extend(parse_linked_artifact_entries(
            artifact,
            "zsh_completion",
            CaskLinkedArtifactKind::ZshCompletion,
            zsh_completion_target,
        )?);
    }

    Ok(linked_artifacts)
}

fn artifacts(cask: &Value) -> Result<&Vec<Value>, Error> {
    cask.get("artifacts")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::InvalidArgument {
            message: "failed to parse cask JSON: missing artifacts array".to_string(),
        })
}

fn parse_linked_artifact_entries(
    artifact: &Value,
    key: &str,
    kind: CaskLinkedArtifactKind,
    target_for: fn(&str, Option<&str>) -> Result<String, Error>,
) -> Result<Vec<CaskLinkedArtifact>, Error> {
    let Some(entries) = artifact.get(key).and_then(Value::as_array) else {
        return Ok(Vec::new());
    };

    entries
        .iter()
        .map(|entry| {
            let (source, target) = parse_symlink_entry(entry)?;
            let explicit_target = target.as_deref().or_else(|| artifact_target(artifact));
            let target = linked_artifact_target(&source, explicit_target, target_for)?;
            Ok(CaskLinkedArtifact {
                kind: kind.clone(),
                source,
                target,
            })
        })
        .collect()
}

fn is_flat_artifact_entry(entries: &[Value]) -> bool {
    entries.len() == 2 && entries[0].is_string() && entries[1].is_object()
}

fn artifact_target(artifact: &Value) -> Option<&str> {
    artifact.get("target").and_then(Value::as_str)
}

fn parse_binary_entry(
    entry: &Value,
    fallback_target: Option<&str>,
) -> Result<(String, String), Error> {
    let (source, target) = parse_artifact_entry(entry, "binary")?;
    let target = target.as_deref().or(fallback_target);
    let target = match target {
        Some(target) => normalize_binary_target(target)?,
        None => format!("bin/{}", basename(source)?),
    };
    Ok((source.to_string(), target))
}

fn parse_app_entry(
    entry: &Value,
    fallback_target: Option<&str>,
) -> Result<(String, String), Error> {
    let (source, target) = parse_artifact_entry(entry, "app")?;
    let target = target.as_deref().or(fallback_target);
    let target = match target {
        Some(target) => normalize_app_target(target)?,
        None => basename(source)?,
    };
    Ok((source.to_string(), target))
}

fn parse_symlink_entry(entry: &Value) -> Result<(String, Option<String>), Error> {
    let (source, target) = parse_artifact_entry(entry, "symlink")?;
    Ok((source.to_string(), target))
}

fn parse_artifact_entry<'a>(
    entry: &'a Value,
    artifact_kind: &str,
) -> Result<(&'a str, Option<String>), Error> {
    if let Some(path) = entry.as_str() {
        return Ok((path, None));
    }

    let array = entry.as_array().ok_or_else(|| Error::InvalidArgument {
        message: format!("unsupported cask {artifact_kind} artifact shape"),
    })?;
    let source = array
        .first()
        .and_then(Value::as_str)
        .ok_or_else(|| Error::InvalidArgument {
            message: format!("unsupported cask {artifact_kind} source"),
        })?;
    let target = array
        .get(1)
        .and_then(Value::as_object)
        .and_then(|obj| obj.get("target"))
        .and_then(Value::as_str)
        .map(ToString::to_string);

    Ok((source, target))
}

fn linked_artifact_target(
    source: &str,
    target: Option<&str>,
    target_for: fn(&str, Option<&str>) -> Result<String, Error>,
) -> Result<String, Error> {
    match target {
        Some(target)
            if target.contains('/')
                || target.starts_with("$HOMEBREW_PREFIX")
                || std::path::Path::new(target).is_absolute() =>
        {
            normalize_prefix_target(target, "linked artifact")
        }
        _ => target_for(source, target),
    }
}

fn target_or_basename(source: &str, target: Option<&str>) -> String {
    target
        .map(ToString::to_string)
        .unwrap_or_else(|| basename(source).unwrap_or_else(|_| source.to_string()))
}

fn manpage_target(source: &str, target: Option<&str>) -> Result<String, Error> {
    let target = target_or_basename(source, target);
    let section = manpage_section(&target).or_else(|| manpage_section(source));
    let section = section.ok_or_else(|| Error::InvalidArgument {
        message: format!("failed to determine manpage section for '{source}'"),
    })?;
    Ok(format!("share/man/man{section}/{target}"))
}

fn bash_completion_target(source: &str, target: Option<&str>) -> Result<String, Error> {
    let target = target
        .map(ToString::to_string)
        .unwrap_or_else(|| completion_stem(source));
    Ok(format!("etc/bash_completion.d/{target}"))
}

fn fish_completion_target(source: &str, target: Option<&str>) -> Result<String, Error> {
    let mut target = target_or_basename(source, target);
    if !target.ends_with(".fish") {
        target.push_str(".fish");
    }
    Ok(format!("share/fish/vendor_completions.d/{target}"))
}

fn zsh_completion_target(source: &str, target: Option<&str>) -> Result<String, Error> {
    let mut target = target_or_basename(source, target);
    if !target.starts_with('_') {
        target.insert(0, '_');
    }
    Ok(format!("share/zsh/site-functions/{target}"))
}

fn manpage_section(path: &str) -> Option<String> {
    let name = std::path::Path::new(path).file_name()?.to_str()?;
    let without_gz = name.strip_suffix(".gz").unwrap_or(name);
    let section = without_gz.rsplit_once('.')?.1;
    if section == "n"
        || section == "l"
        || section
            .chars()
            .next()
            .map(|ch| ch.is_ascii_digit())
            .unwrap_or(false)
    {
        Some(section.to_string())
    } else {
        None
    }
}

fn completion_stem(path: &str) -> String {
    let name = basename(path).unwrap_or_else(|_| path.to_string());
    std::path::Path::new(&name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(ToString::to_string)
        .unwrap_or(name)
}

fn normalize_binary_target(target: &str) -> Result<String, Error> {
    let target = normalize_prefix_target(target, "binary")?;
    if target.contains('/') {
        Ok(target)
    } else {
        Ok(format!("bin/{target}"))
    }
}

fn normalize_app_target(target: &str) -> Result<String, Error> {
    let target = target
        .strip_prefix("/Applications/")
        .or_else(|| target.strip_prefix("$APPDIR/"))
        .unwrap_or(target);
    validate_leaf_target(target, "app")
}

fn normalize_prefix_target(target: &str, artifact_kind: &str) -> Result<String, Error> {
    let target = target
        .strip_prefix("$HOMEBREW_PREFIX/")
        .or_else(|| target.strip_prefix("/usr/local/"))
        .or_else(|| target.strip_prefix("/opt/homebrew/"))
        .unwrap_or(target);
    let target_path = std::path::Path::new(target);
    if target.is_empty()
        || target_path.is_absolute()
        || target.contains('$')
        || target.contains('~')
        || target_path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return Err(Error::InvalidArgument {
            message: format!("unsupported cask {artifact_kind} target path '{target}'"),
        });
    }

    Ok(target.to_string())
}

fn validate_leaf_target(target: &str, artifact_kind: &str) -> Result<String, Error> {
    let normalized = normalize_prefix_target(target, artifact_kind)?;
    if normalized.contains('/') {
        return Err(Error::InvalidArgument {
            message: format!("unsupported cask {artifact_kind} target path '{target}'"),
        });
    }
    Ok(normalized)
}

fn step_path<'a>(step: &'a Value, key: &str) -> Option<&'a str> {
    step.get(key)?.get("path")?.as_str()
}

fn parse_postflight_symlinks(cask: &Value) -> Result<Vec<CaskPostflightSymlink>, Error> {
    let mut symlinks = Vec::new();

    for artifact in artifacts(cask)? {
        let Some(blocks) = artifact.get("postflight_steps").and_then(Value::as_array) else {
            continue;
        };
        for block in blocks {
            let Some(steps) = block.get("steps").and_then(Value::as_array) else {
                continue;
            };
            for step in steps {
                if step.get("type").and_then(Value::as_str) != Some("symlink") {
                    continue;
                }
                let (Some(source), Some(target)) =
                    (step_path(step, "source"), step_path(step, "target"))
                else {
                    continue;
                };
                let source = source
                    .strip_prefix("{{appdir}}")
                    .map(|rest| format!("$APPDIR{rest}"))
                    .unwrap_or_else(|| source.to_string());
                let skip_if_exists = step
                    .get("guards")
                    .and_then(Value::as_array)
                    .map(|guards| {
                        guards.iter().any(|guard| {
                            guard.get("condition").and_then(Value::as_str) == Some("unless_exists")
                        })
                    })
                    .unwrap_or(false);

                symlinks.push(CaskPostflightSymlink {
                    source,
                    target: normalize_prefix_target(target, "postflight symlink")?,
                    skip_if_exists,
                    uninstall: step
                        .get("uninstall")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                });
            }
        }
    }

    Ok(symlinks)
}

fn basename(path: &str) -> Result<String, Error> {
    let name = std::path::Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| Error::InvalidArgument {
            message: format!("invalid cask binary path '{path}'"),
        })?;
    Ok(name.to_string())
}

#[cfg(all(test, target_os = "macos"))]
#[path = "cask/tests.rs"]
mod tests;
