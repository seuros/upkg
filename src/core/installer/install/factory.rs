use std::fs::File;
use std::path::Path;

use crate::core::cellar::link::Linker;
use crate::core::cellar::materialize::Cellar;
use crate::core::network::api::ApiClient;
use crate::core::network::download::ParallelDownloader;
use crate::core::storage::blob::BlobCache;
use crate::core::storage::receipt::scan_installed;
use crate::core::storage::state_db::{InstalledPackage, InstalledPackageKind, StateDb};
use crate::core::storage::store::Store;
use crate::package_ref::{cask_name, is_cask_name};
use crate::types::Error;

use super::Installer;

pub fn create_installer(
    root: &Path,
    prefix: &Path,
    concurrency: usize,
) -> Result<Installer, Error> {
    use std::fs;

    if !root.exists() {
        fs::create_dir_all(root).map_err(|e| {
            if e.kind() == std::io::ErrorKind::PermissionDenied {
                Error::StoreCorruption {
                    message: format!(
                        "cannot create root directory '{}': permission denied.\n\n\
                        Create it with:\n  sudo mkdir -p {} && sudo chown $USER {}",
                        root.display(),
                        root.display(),
                        root.display()
                    ),
                }
            } else {
                Error::StoreCorruption {
                    message: format!("failed to create root directory '{}': {e}", root.display()),
                }
            }
        })?;
    }

    let lock_file = acquire_global_lock(root)?;

    let api_client = ApiClient::new();
    let blob_cache = BlobCache::new(&root.join("cache")).map_err(|e| Error::StoreCorruption {
        message: format!("failed to create blob cache: {e}"),
    })?;
    let store = Store::new(root).map_err(|e| Error::StoreCorruption {
        message: format!("failed to create store: {e}"),
    })?;
    let state_db = StateDb::open(&root.join("db").join("upkg.sqlite3"))?;
    backfill_state_db_if_empty(&state_db, prefix)?;
    let cellar = Cellar::new_at(prefix.join("Cellar")).map_err(|e| Error::StoreCorruption {
        message: format!("failed to create cellar: {e}"),
    })?;
    let linker = Linker::new(prefix).map_err(|e| Error::StoreCorruption {
        message: format!("failed to create linker: {e}"),
    })?;
    let parallel_downloader = ParallelDownloader::with_concurrency(blob_cache, concurrency);

    Ok(Installer {
        api_client,
        downloader: parallel_downloader,
        store,
        cellar,
        linker,
        state_db,
        prefix: prefix.to_path_buf(),
        _lock: Some(lock_file),
    })
}

fn acquire_global_lock(root: &Path) -> Result<File, Error> {
    let locks_dir = root.join("locks");
    std::fs::create_dir_all(&locks_dir).map_err(|e| Error::StoreCorruption {
        message: format!("failed to create lock directory: {e}"),
    })?;

    let lock_path = locks_dir.join("upkg.lock");
    let lock_file = File::create(&lock_path).map_err(|e| Error::StoreCorruption {
        message: format!("failed to create global lock file: {e}"),
    })?;

    if let Err(err) = lock_file.try_lock() {
        let std::fs::TryLockError::WouldBlock = err else {
            return Err(Error::StoreCorruption {
                message: format!("failed to acquire global lock: {err}"),
            });
        };

        eprintln!("    Waiting for another upkg process to finish...");
        lock_file.lock().map_err(|e| Error::StoreCorruption {
            message: format!("failed to acquire global lock: {e}"),
        })?;
    }

    Ok(lock_file)
}

fn backfill_state_db_if_empty(state_db: &StateDb, prefix: &Path) -> Result<(), Error> {
    if state_db.has_installed_packages()? {
        return Ok(());
    }

    let installed_at = current_timestamp();
    for keg in scan_installed(&prefix.join("Cellar"))? {
        state_db.record_installed(&InstalledPackage {
            kind: if is_cask_name(&keg.name) {
                InstalledPackageKind::App
            } else {
                InstalledPackageKind::Formula
            },
            name: keg.name.clone(),
            formula_name: keg.name,
            version: keg.version,
            store_key: keg.store_key,
            installed_at,
        })?;
    }

    for (token, version) in scan_caskroom(prefix)? {
        let name = cask_name(&token);
        state_db.record_installed(&InstalledPackage {
            name: name.clone(),
            formula_name: name,
            version,
            store_key: String::new(),
            kind: InstalledPackageKind::App,
            installed_at,
        })?;
    }

    Ok(())
}

fn scan_caskroom(prefix: &Path) -> Result<Vec<(String, String)>, Error> {
    let caskroom = prefix.join("Caskroom");
    if !caskroom.exists() {
        return Ok(Vec::new());
    }

    let mut out = Vec::new();
    for entry in std::fs::read_dir(&caskroom).map_err(|e| Error::StoreCorruption {
        message: format!("failed to read Caskroom '{}': {e}", caskroom.display()),
    })? {
        let path = match entry {
            Ok(entry) => entry.path(),
            Err(_) => continue,
        };
        if !path.is_dir() {
            continue;
        }
        let Some(token) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(version) = latest_cask_version(&path)? else {
            continue;
        };
        out.push((token.to_string(), version));
    }

    Ok(out)
}

fn latest_cask_version(cask_path: &Path) -> Result<Option<String>, Error> {
    let mut versions = Vec::new();
    for entry in std::fs::read_dir(cask_path).map_err(|e| Error::StoreCorruption {
        message: format!("failed to read cask path '{}': {e}", cask_path.display()),
    })? {
        let path = match entry {
            Ok(entry) => entry.path(),
            Err(_) => continue,
        };
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name == ".metadata" || !path.is_dir() {
            continue;
        }
        versions.push(name.to_string());
    }
    versions.sort();
    Ok(versions.pop())
}

fn current_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
