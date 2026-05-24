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
pub struct ResolvedCask {
    pub install_name: String,
    pub token: String,
    pub version: String,
    pub url: String,
    pub sha256: String,
    pub binaries: Vec<CaskBinary>,
    pub apps: Vec<CaskApp>,
    pub linked_artifacts: Vec<CaskLinkedArtifact>,
}

pub fn resolve_cask(token: &str, cask: &Value) -> Result<ResolvedCask, Error> {
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
    let linked_artifacts = parse_linked_artifacts(cask)?;
    if binaries.is_empty() && apps.is_empty() {
        return Err(Error::InvalidArgument {
            message: format!("cask '{token}' does not expose supported app or binary artifacts"),
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
        linked_artifacts,
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

        for entry in entries {
            let (source, target) = parse_binary_entry(entry)?;
            binaries.push(CaskBinary { source, target });
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

        for entry in entries {
            let (source, target) = parse_binary_entry(entry)?;
            apps.push(CaskApp { source, target });
        }
    }

    Ok(apps)
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
            let target = target_for(&source, target.as_deref())?;
            Ok(CaskLinkedArtifact {
                kind: kind.clone(),
                source,
                target,
            })
        })
        .collect()
}

fn parse_binary_entry(entry: &Value) -> Result<(String, String), Error> {
    let (source, target) = parse_artifact_entry(entry, "binary")?;
    let target = target.unwrap_or_else(|| basename(source).unwrap_or_else(|_| source.to_string()));
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
        .map(|target| validate_relative_target(target, artifact_kind))
        .transpose()?;

    Ok((source, target))
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

fn validate_relative_target(target: &str, artifact_kind: &str) -> Result<String, Error> {
    if target.contains('/') || target.contains('$') || target.contains('~') {
        return Err(Error::InvalidArgument {
            message: format!("unsupported cask {artifact_kind} target path '{target}'"),
        });
    }

    Ok(target.to_string())
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
        assert_eq!(resolved.binaries[0].target, "tool");
        assert_eq!(resolved.binaries[1].target, "tool-two");
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
