use std::fs;
use std::path::{Path, PathBuf};

use crate::types::{Error, formula_token};

#[derive(Debug, Clone)]
pub struct InstalledKeg {
    pub name: String,
    pub version: String,
    pub store_key: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InstallReceipt {
    pub install_name: String,
    pub formula_name: String,
    pub version: String,
    pub store_key: String,
    pub installed_at: i64,
}

pub fn write_receipt(keg_path: &Path, receipt: &InstallReceipt) -> Result<(), Error> {
    let path = receipt_path(keg_path);
    let data = serde_json::to_vec_pretty(receipt).map_err(|e| Error::StoreCorruption {
        message: format!("failed to serialize INSTALL_RECEIPT.json: {e}"),
    })?;
    fs::write(&path, data).map_err(|e| Error::StoreCorruption {
        message: format!("failed to write {}: {e}", path.display()),
    })
}

pub fn read_receipt(keg_path: &Path) -> Option<InstallReceipt> {
    let path = receipt_path(keg_path);
    let data = fs::read(path).ok()?;
    serde_json::from_slice::<InstallReceipt>(&data).ok()
}

pub fn receipt_path(keg_path: &Path) -> PathBuf {
    keg_path.join("INSTALL_RECEIPT.json")
}

pub fn scan_installed(cellar: &Path) -> Result<Vec<InstalledKeg>, Error> {
    if !cellar.exists() {
        return Ok(Vec::new());
    }

    let mut out = Vec::new();
    for formula_dir in fs::read_dir(cellar).map_err(|e| Error::StoreCorruption {
        message: format!(
            "failed to read cellar directory '{}': {e}",
            cellar.display()
        ),
    })? {
        let formula_dir = match formula_dir {
            Ok(v) => v.path(),
            Err(_) => continue,
        };
        if !formula_dir.is_dir() {
            continue;
        }

        let formula_name = formula_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();

        for version_dir in fs::read_dir(&formula_dir).map_err(|e| Error::StoreCorruption {
            message: format!(
                "failed to read formula directory '{}': {e}",
                formula_dir.display()
            ),
        })? {
            let version_dir = match version_dir {
                Ok(v) => v.path(),
                Err(_) => continue,
            };
            if !version_dir.is_dir() {
                continue;
            }

            let version = version_dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();

            if let Some(receipt) = read_receipt(&version_dir) {
                out.push(InstalledKeg {
                    name: receipt.install_name,
                    version: receipt.version,
                    store_key: receipt.store_key,
                });
            } else {
                out.push(InstalledKeg {
                    name: formula_name.clone(),
                    version,
                    store_key: String::new(),
                });
            }
        }
    }

    out.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.version.cmp(&b.version)));
    Ok(out)
}

pub fn find_installed(cellar: &Path, name: &str) -> Option<InstalledKeg> {
    let installed = scan_installed(cellar).ok()?;
    if name.contains('/') {
        return installed.into_iter().find(|keg| keg.name == name);
    }

    let needle = formula_token(name).to_string();
    installed.into_iter().find(|keg| {
        keg.name == name || (!keg.name.contains('/') && formula_token(&keg.name) == needle)
    })
}
