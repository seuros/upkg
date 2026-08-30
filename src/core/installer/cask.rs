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
        if let Some(variation_url) = variation.get("url").and_then(Value::as_str) {
            url = variation_url.to_string();
        }
        if let Some(variation_sha) = variation.get("sha256").and_then(Value::as_str) {
            sha256 = variation_sha.to_string();
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

fn manpage_target(source: &str, target: Option<&str>) -> Result<String, Error> {
    let target = target
        .map(ToString::to_string)
        .unwrap_or_else(|| basename(source).unwrap_or_else(|_| source.to_string()));
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
    let mut target = target
        .map(ToString::to_string)
        .unwrap_or_else(|| basename(source).unwrap_or_else(|_| source.to_string()));
    if !target.ends_with(".fish") {
        target.push_str(".fish");
    }
    Ok(format!("share/fish/vendor_completions.d/{target}"))
}

fn zsh_completion_target(source: &str, target: Option<&str>) -> Result<String, Error> {
    let mut target = target
        .map(ToString::to_string)
        .unwrap_or_else(|| basename(source).unwrap_or_else(|_| source.to_string()));
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
                let Some(source) = step
                    .get("source")
                    .and_then(|value| value.get("path"))
                    .and_then(Value::as_str)
                else {
                    continue;
                };
                let Some(target) = step
                    .get("target")
                    .and_then(|value| value.get("path"))
                    .and_then(Value::as_str)
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
mod tests {
    use super::*;

    #[test]
    fn resolve_cask_uses_platform_variation_url_and_sha() {
        let mut cask = serde_json::json!({
            "token": "test",
            "version": "1.0.0",
            "url": "https://example.com/darwin.zip",
            "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "artifacts": [{ "binary": [["op"]] }],
            "variations": {}
        });
        let variation_key = current_macos_variation_key();
        cask["variations"][variation_key.as_str()] = serde_json::json!({
            "url": "https://example.com/macos.zip",
            "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        });

        let resolved = resolve_cask("test", &cask).unwrap();
        assert_eq!(resolved.url, "https://example.com/macos.zip");
        assert_eq!(
            resolved.sha256,
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        );
    }

    #[test]
    fn resolve_cask_parses_binary_targets() {
        let cask = serde_json::json!({
            "token": "test",
            "version": "1.0.0",
            "url": "https://example.com/test.zip",
            "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "artifacts": [{
                "binary": [
                    ["bin/tool"],
                    ["bin/tool2", {"target": "tool-two"}]
                ]
            }]
        });

        let resolved = resolve_cask("test", &cask).unwrap();
        assert_eq!(resolved.binaries.len(), 2);
        assert_eq!(resolved.binaries[0].target, "bin/tool");
        assert_eq!(resolved.binaries[1].target, "bin/tool-two");
    }

    #[test]
    fn resolve_cask_parses_app_artifacts() {
        let cask = serde_json::json!({
            "token": "ghostty",
            "version": "1.0.0",
            "url": "https://example.com/Ghostty.dmg",
            "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "artifacts": [{
                "app": ["Ghostty.app"]
            }]
        });

        let resolved = resolve_cask("ghostty", &cask).unwrap();
        assert!(resolved.binaries.is_empty());
        assert_eq!(resolved.apps.len(), 1);
        assert_eq!(resolved.apps[0].source, "Ghostty.app");
        assert_eq!(resolved.apps[0].target, "Ghostty.app");
    }

    #[test]
    fn resolve_cask_parses_pkg_artifacts_and_uninstall_directives() {
        let cask = serde_json::json!({
            "token": "test-pkg",
            "version": "1.0.0",
            "url": "https://example.com/Test.dmg",
            "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "artifacts": [
                { "uninstall": [{
                    "pkgutil": ["com.example.test", "com.example.helper"],
                    "delete": ["/Library/Application Support/Test"]
                }] },
                { "pkg": ["Test Installer.pkg"] }
            ]
        });

        let resolved = resolve_cask("test-pkg", &cask).unwrap();

        assert!(resolved.apps.is_empty());
        assert!(resolved.binaries.is_empty());
        assert_eq!(resolved.pkgs.len(), 1);
        assert_eq!(resolved.pkgs[0].source, "Test Installer.pkg");
        assert_eq!(
            resolved.uninstall.pkgutil,
            vec!["com.example.test", "com.example.helper"]
        );
        assert_eq!(
            resolved.uninstall.delete,
            vec!["/Library/Application Support/Test"]
        );
    }

    #[test]
    fn resolve_cask_parses_string_pkgutil_uninstall_directive() {
        let cask = serde_json::json!({
            "token": "test-pkg",
            "version": "1.0.0",
            "url": "https://example.com/Test.pkg",
            "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "artifacts": [
                { "uninstall": [{ "pkgutil": "com.example.*" }] },
                { "pkg": ["Test.pkg", { "allow_untrusted": true }] }
            ]
        });

        let resolved = resolve_cask("test-pkg", &cask).unwrap();

        assert_eq!(resolved.pkgs.len(), 1);
        assert_eq!(resolved.pkgs[0].source, "Test.pkg");
        assert_eq!(resolved.uninstall.pkgutil, vec!["com.example.*"]);
    }

    #[test]
    fn resolve_cask_parses_secondary_artifacts() {
        let cask = serde_json::json!({
            "token": "ghostty",
            "version": "1.0.0",
            "url": "https://example.com/Ghostty.dmg",
            "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "artifacts": [
                { "app": ["Ghostty.app"] },
                { "manpage": ["$APPDIR/Ghostty.app/Contents/Resources/man/man1/ghostty.1"] },
                { "manpage": ["$APPDIR/Ghostty.app/Contents/Resources/man/man5/ghostty.5"] },
                { "bash_completion": ["$APPDIR/Ghostty.app/Contents/Resources/bash-completion/completions/ghostty.bash"] },
                { "fish_completion": ["$APPDIR/Ghostty.app/Contents/Resources/fish/vendor_completions.d/ghostty.fish"] },
                { "zsh_completion": ["$APPDIR/Ghostty.app/Contents/Resources/zsh/site-functions/_ghostty"] }
            ]
        });

        let resolved = resolve_cask("ghostty", &cask).unwrap();
        let targets: Vec<_> = resolved
            .linked_artifacts
            .iter()
            .map(|artifact| artifact.target.as_str())
            .collect();
        assert_eq!(
            targets,
            vec![
                "share/man/man1/ghostty.1",
                "share/man/man5/ghostty.5",
                "etc/bash_completion.d/ghostty",
                "share/fish/vendor_completions.d/ghostty.fish",
                "share/zsh/site-functions/_ghostty"
            ]
        );
    }

    #[test]
    fn resolve_cask_parses_flat_binary_artifact() {
        let cask = serde_json::json!({
            "token": "gimp",
            "version": "3.2.4",
            "url": "https://example.com/gimp.dmg",
            "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "artifacts": [
                { "app": ["GIMP.app"] },
                { "binary": [
                    "$HOMEBREW_PREFIX/Caskroom/gimp/3.2.4/gimp.wrapper.sh",
                    { "target": "gimp" }
                ]}
            ]
        });

        let resolved = resolve_cask("gimp", &cask).unwrap();
        assert_eq!(resolved.apps.len(), 1);
        assert_eq!(resolved.apps[0].source, "GIMP.app");
        assert_eq!(resolved.binaries.len(), 1);
        assert_eq!(
            resolved.binaries[0].source,
            "$HOMEBREW_PREFIX/Caskroom/gimp/3.2.4/gimp.wrapper.sh"
        );
        assert_eq!(resolved.binaries[0].target, "bin/gimp");
    }

    #[test]
    fn resolve_cask_parses_flat_app_artifact_with_target() {
        let cask = serde_json::json!({
            "token": "test",
            "version": "1.0.0",
            "url": "https://example.com/test.dmg",
            "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "artifacts": [{
                "app": ["MyApp.app", { "target": "Custom.app" }]
            }]
        });

        let resolved = resolve_cask("test", &cask).unwrap();
        assert_eq!(resolved.apps.len(), 1);
        assert_eq!(resolved.apps[0].source, "MyApp.app");
        assert_eq!(resolved.apps[0].target, "Custom.app");
    }

    #[test]
    fn resolve_cask_parses_multiple_binary_entries() {
        let cask = serde_json::json!({
            "token": "test",
            "version": "1.0.0",
            "url": "https://example.com/test.zip",
            "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "artifacts": [{
                "binary": [
                    ["bin/tool"],
                    ["bin/tool2", {"target": "tool-two"}]
                ]
            }]
        });

        let resolved = resolve_cask("test", &cask).unwrap();
        assert_eq!(resolved.binaries.len(), 2);
        assert_eq!(resolved.binaries[0].source, "bin/tool");
        assert_eq!(resolved.binaries[0].target, "bin/tool");
        assert_eq!(resolved.binaries[1].source, "bin/tool2");
        assert_eq!(resolved.binaries[1].target, "bin/tool-two");
    }

    #[test]
    fn resolve_cask_parses_docker_desktop_artifacts() {
        let cask = serde_json::json!({
            "token": "docker-desktop",
            "old_tokens": ["docker"],
            "version": "4.88.1,237512",
            "url": "https://example.com/Docker.dmg",
            "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "artifacts": [
                {
                    "app": ["Docker.app"],
                    "target": "/Applications/Docker.app"
                },
                {
                    "binary": [
                        "$APPDIR/Docker.app/Contents/Resources/bin/docker",
                        {"target": "/usr/local/bin/docker"}
                    ],
                    "target": "/usr/local/bin/docker"
                },
                {
                    "binary": [
                        "$APPDIR/Docker.app/Contents/Resources/cli-plugins/docker-compose",
                        {"target": "/usr/local/cli-plugins/docker-compose"}
                    ],
                    "target": "/usr/local/cli-plugins/docker-compose"
                },
                {
                    "fish_completion": [
                        "$APPDIR/Docker.app/Contents/Resources/etc/docker.fish-completion"
                    ],
                    "target": "$HOMEBREW_PREFIX/share/fish/vendor_completions.d/docker.fish"
                },
                {
                    "zsh_completion": [
                        "$APPDIR/Docker.app/Contents/Resources/etc/docker.zsh-completion"
                    ],
                    "target": "$HOMEBREW_PREFIX/share/zsh/site-functions/_docker"
                },
                {
                    "postflight_steps": [{
                        "steps": [{
                            "source": {
                                "path": "{{appdir}}/Docker.app/Contents/Resources/bin/kubectl"
                            },
                            "target": {"path": "/usr/local/bin/kubectl"},
                            "uninstall": true,
                            "guards": [{
                                "path": "/usr/local/bin/kubectl",
                                "condition": "unless_exists"
                            }],
                            "type": "symlink"
                        }]
                    }]
                }
            ]
        });

        let resolved = resolve_cask("docker", &cask).unwrap();

        assert_eq!(resolved.token, "docker-desktop");
        assert_eq!(resolved.install_name, "cask:docker-desktop");
        assert_eq!(resolved.apps[0].target, "Docker.app");
        assert_eq!(resolved.binaries[0].target, "bin/docker");
        assert_eq!(resolved.binaries[1].target, "cli-plugins/docker-compose");
        assert_eq!(
            resolved.linked_artifacts[0].target,
            "share/fish/vendor_completions.d/docker.fish"
        );
        assert_eq!(
            resolved.linked_artifacts[1].target,
            "share/zsh/site-functions/_docker"
        );
        assert_eq!(resolved.postflight_symlinks.len(), 1);
        assert_eq!(
            resolved.postflight_symlinks[0].source,
            "$APPDIR/Docker.app/Contents/Resources/bin/kubectl"
        );
        assert_eq!(resolved.postflight_symlinks[0].target, "bin/kubectl");
        assert!(resolved.postflight_symlinks[0].skip_if_exists);
        assert!(resolved.postflight_symlinks[0].uninstall);
    }

    #[test]
    fn resolve_cask_rejects_binary_targets_outside_known_prefixes() {
        let cask = serde_json::json!({
            "token": "test",
            "version": "1.0.0",
            "url": "https://example.com/test.zip",
            "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "artifacts": [{
                "binary": ["bin/tool", {"target": "/tmp/tool"}]
            }]
        });

        let err = resolve_cask("test", &cask).unwrap_err();
        assert!(matches!(err, Error::InvalidArgument { .. }));
    }

    #[test]
    fn resolve_cask_missing_required_field_is_invalid_argument() {
        let cask = serde_json::json!({
            "token": "test",
            "version": "1.0.0",
            "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "artifacts": [{ "binary": [["op"]] }]
        });

        let err = resolve_cask("test", &cask).unwrap_err();
        assert!(matches!(err, Error::InvalidArgument { .. }));
    }

    #[test]
    fn resolve_cask_missing_artifacts_array_is_invalid_argument() {
        let cask = serde_json::json!({
            "token": "test",
            "version": "1.0.0",
            "url": "https://example.com/test.zip",
            "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        });

        let err = resolve_cask("test", &cask).unwrap_err();
        assert!(matches!(err, Error::InvalidArgument { .. }));
    }
}
