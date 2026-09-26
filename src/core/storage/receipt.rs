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
            if !version_dir.is_dir() || is_keg_backup(&version_dir) {
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
            } else if is_populated(&version_dir) {
                // Adopted keg (e.g. left by Homebrew): no upkg receipt, but real contents.
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

/// Kegs set aside while a source build or reinstall runs; they are restored or
/// removed afterwards and must never be reported as installed.
fn is_keg_backup(version_dir: &Path) -> bool {
    version_dir
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.contains(".upkg-backup-"))
}

/// Anything besides a receipt counts; an empty directory is debris from an
/// install that failed before writing files, not an installed keg.
fn is_populated(keg_path: &Path) -> bool {
    fs::read_dir(keg_path)
        .map(|mut entries| entries.next().is_some())
        .unwrap_or(false)
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn empty_version_dir_is_not_installed() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("gmp/6.3.0")).unwrap();

        assert!(scan_installed(tmp.path()).unwrap().is_empty());
        assert!(find_installed(tmp.path(), "gmp").is_none());
    }

    #[test]
    fn populated_keg_without_receipt_is_adopted() {
        let tmp = TempDir::new().unwrap();
        let lib = tmp.path().join("libyaml/0.2.5/lib");
        fs::create_dir_all(&lib).unwrap();
        fs::write(lib.join("libyaml-0.2.dylib"), b"").unwrap();

        let keg = find_installed(tmp.path(), "libyaml").expect("adopted keg");
        assert_eq!(keg.version, "0.2.5");
        assert!(keg.store_key.is_empty());
    }

    #[test]
    fn keg_backups_are_not_installed() {
        let tmp = TempDir::new().unwrap();
        let lib = tmp.path().join("jq/1.8.1.upkg-backup-42/lib");
        fs::create_dir_all(&lib).unwrap();
        fs::write(lib.join("libjq.1.dylib"), b"").unwrap();

        assert!(scan_installed(tmp.path()).unwrap().is_empty());
    }

    #[test]
    fn receipt_wins_over_directory_name() {
        let tmp = TempDir::new().unwrap();
        let keg_path = tmp.path().join("openssl@3/3.6.2");
        fs::create_dir_all(&keg_path).unwrap();
        write_receipt(
            &keg_path,
            &InstallReceipt {
                install_name: "openssl@3".into(),
                formula_name: "openssl@3".into(),
                version: "3.6.2".into(),
                store_key: "abc".into(),
                installed_at: 0,
            },
        )
        .unwrap();

        let keg = find_installed(tmp.path(), "openssl@3").unwrap();
        assert_eq!(keg.store_key, "abc");
    }
}
