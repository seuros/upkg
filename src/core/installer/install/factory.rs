use std::fs::File;
use std::path::Path;

use crate::clock::unix_secs;
use crate::core::cellar::link::Linker;
use crate::core::cellar::materialize::Cellar;
use crate::core::network::api::ApiClient;
use crate::core::network::download::ParallelDownloader;
use crate::core::storage::blob::BlobCache;
use crate::core::storage::fs_util::{StoreContext, dir_paths, file_name_str};
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
    let blob_cache =
        BlobCache::new(&root.join("cache")).store_context(|| "failed to create blob cache")?;
    let store = Store::new(root).store_context(|| "failed to create store")?;
    let state_db = StateDb::open(&root.join("db").join("upkg.sqlite3"))?;
    backfill_state_db_if_empty(&state_db, prefix)?;
    let cellar =
        Cellar::new_at(prefix.join("Cellar")).store_context(|| "failed to create cellar")?;
    let linker = Linker::new(prefix).store_context(|| "failed to create linker")?;
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
    std::fs::create_dir_all(&locks_dir).store_context(|| "failed to create lock directory")?;

    let lock_path = locks_dir.join("upkg.lock");
    let lock_file =
        File::create(&lock_path).store_context(|| "failed to create global lock file")?;

    if let Err(err) = lock_file.try_lock() {
        let std::fs::TryLockError::WouldBlock = err else {
            return Err(Error::StoreCorruption {
                message: format!("failed to acquire global lock: {err}"),
            });
        };

        eprintln!("    Waiting for another upkg process to finish...");
        lock_file
            .lock()
            .store_context(|| "failed to acquire global lock")?;
    }

    Ok(lock_file)
}

fn backfill_state_db_if_empty(state_db: &StateDb, prefix: &Path) -> Result<(), Error> {
    if state_db.has_installed_packages()? {
        return Ok(());
    }

    let installed_at = unix_secs();
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
    for path in dir_paths(&caskroom, "Caskroom")?.filter(|path| path.is_dir()) {
        let Some(token) = file_name_str(&path) else {
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
    for path in dir_paths(cask_path, "cask path")? {
        let Some(name) = file_name_str(&path) else {
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
