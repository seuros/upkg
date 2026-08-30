use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::types::Error;

pub(in crate::core::installer::install) fn write_brew_cask_metadata(
    caskroom_path: &Path,
    cask: &crate::core::installer::cask::ResolvedCask,
    cask_json: &serde_json::Value,
) -> Result<(), Error> {
    let metadata_dir = caskroom_path.join(".metadata");
    let timestamp = current_brew_timestamp();
    let caskfile_dir = metadata_dir
        .join(&cask.version)
        .join(&timestamp)
        .join("Casks");
    fs::create_dir_all(&caskfile_dir).map_err(|e| Error::StoreCorruption {
        message: format!("failed to create cask metadata directory: {e}"),
    })?;

    write_json_pretty(
        &caskfile_dir.join(format!("{}.json", cask.token)),
        cask_json,
    )?;
    write_json_pretty(&metadata_dir.join("config.json"), &brew_cask_config_json())?;
    write_json_pretty(
        &metadata_dir.join("INSTALL_RECEIPT.json"),
        &brew_cask_receipt_json(cask, cask_json),
    )?;

    Ok(())
}

fn current_brew_timestamp() -> String {
    let output = Command::new("date")
        .args(["-u", "+%Y%m%d%H%M%S.000"])
        .output();

    output
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "19700101000000.000".to_string())
}

fn brew_cask_config_json() -> serde_json::Value {
    serde_json::json!({
        "default": {
            "appdir": "/Applications"
        },
        "env": {},
        "explicit": {}
    })
}

fn brew_cask_receipt_json(
    cask: &crate::core::installer::cask::ResolvedCask,
    cask_json: &serde_json::Value,
) -> serde_json::Value {
    let tap_git_head = cask_json
        .get("tap_git_head")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let ruby_source_path = cask_json
        .get("ruby_source_path")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();

    serde_json::json!({
        "homebrew_version": "4.0.0",
        "loaded_from_api": true,
        "uninstall_flight_blocks": false,
        "installed_as_dependency": false,
        "installed_on_request": true,
        "time": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
        "runtime_dependencies": {},
        "source": {
            "tap": "homebrew/cask",
            "tap_git_head": tap_git_head,
            "version": cask.version,
            "path": ruby_source_path
        },
        "arch": std::env::consts::ARCH,
        "uninstall_artifacts": brew_cask_uninstall_artifacts(cask),
        "built_on": null
    })
}

fn brew_cask_uninstall_artifacts(
    cask: &crate::core::installer::cask::ResolvedCask,
) -> Vec<serde_json::Value> {
    use crate::core::installer::cask::CaskLinkedArtifactKind;

    cask.apps
        .iter()
        .map(|app| serde_json::json!({ "app": [app.target] }))
        .chain(cask.binaries.iter().map(|binary| {
            serde_json::json!({
                "binary": [
                    binary.source,
                    { "target": format!("$HOMEBREW_PREFIX/{}", binary.target) }
                ]
            })
        }))
        .chain(
            cask.pkgs
                .iter()
                .map(|pkg| serde_json::json!({ "pkg": [pkg.source] })),
        )
        .chain(
            std::iter::once(serde_json::json!({
                "uninstall": [{
                    "pkgutil": cask.uninstall.pkgutil.clone(),
                    "delete": cask.uninstall.delete.clone()
                }]
            }))
            .filter(|_| !cask.uninstall.pkgutil.is_empty() || !cask.uninstall.delete.is_empty()),
        )
        .chain(cask.linked_artifacts.iter().map(|artifact| {
            let key = match &artifact.kind {
                CaskLinkedArtifactKind::Manpage => "manpage",
                CaskLinkedArtifactKind::BashCompletion => "bash_completion",
                CaskLinkedArtifactKind::FishCompletion => "fish_completion",
                CaskLinkedArtifactKind::ZshCompletion => "zsh_completion",
            };
            serde_json::json!({ key: [artifact.source.replace("$APPDIR", "/Applications")] })
        }))
        .collect()
}

pub(in crate::core::installer::install) fn write_json_pretty(
    path: &Path,
    value: &serde_json::Value,
) -> Result<(), Error> {
    let data = serde_json::to_vec_pretty(value).map_err(|e| Error::StoreCorruption {
        message: format!("failed to serialize JSON for '{}': {e}", path.display()),
    })?;
    fs::write(path, data).map_err(|e| Error::StoreCorruption {
        message: format!("failed to write '{}': {e}", path.display()),
    })
}

pub(in crate::core::installer::install) fn load_latest_cask_metadata_json(
    caskroom_path: &Path,
    token: &str,
) -> Result<Option<serde_json::Value>, Error> {
    let metadata_dir = caskroom_path.join(".metadata");
    if !metadata_dir.exists() {
        return Ok(None);
    }

    let mut candidates = Vec::new();
    for version_entry in fs::read_dir(&metadata_dir).map_err(|e| Error::StoreCorruption {
        message: format!(
            "failed to read cask metadata directory '{}': {e}",
            metadata_dir.display()
        ),
    })? {
        let version_path = match version_entry {
            Ok(entry) => entry.path(),
            Err(_) => continue,
        };
        if !version_path.is_dir() {
            continue;
        }
        for timestamp_entry in fs::read_dir(&version_path).map_err(|e| Error::StoreCorruption {
            message: format!(
                "failed to read cask metadata version directory '{}': {e}",
                version_path.display()
            ),
        })? {
            let timestamp_path = match timestamp_entry {
                Ok(entry) => entry.path(),
                Err(_) => continue,
            };
            let cask_file = timestamp_path.join("Casks").join(format!("{token}.json"));
            if cask_file.exists() {
                candidates.push(cask_file);
            }
        }
    }

    candidates.sort();
    let Some(path) = candidates.pop() else {
        return Ok(None);
    };

    let data = fs::read_to_string(&path).map_err(|e| Error::StoreCorruption {
        message: format!("failed to read cask metadata '{}': {e}", path.display()),
    })?;
    serde_json::from_str(&data)
        .map(Some)
        .map_err(|e| Error::StoreCorruption {
            message: format!("failed to parse cask metadata '{}': {e}", path.display()),
        })
}

pub(in crate::core::installer::install) fn cask_versions(
    caskroom_path: &Path,
) -> Result<Vec<PathBuf>, Error> {
    let mut versions = Vec::new();
    for entry in fs::read_dir(caskroom_path).map_err(|e| Error::StoreCorruption {
        message: format!(
            "failed to read caskroom path '{}': {e}",
            caskroom_path.display()
        ),
    })? {
        let path = match entry {
            Ok(entry) => entry.path(),
            Err(_) => continue,
        };
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name == ".metadata")
            .unwrap_or(false)
        {
            continue;
        }
        if path.is_dir() {
            versions.push(path);
        }
    }
    Ok(versions)
}
