use std::fs;
use std::path::Path;

#[path = "install/cask_ops.rs"]
mod cask_ops;
#[path = "install/execute.rs"]
mod execute;
#[path = "install/factory.rs"]
mod factory;
#[path = "install/planning.rs"]
mod planning;
#[path = "install/source_ops.rs"]
mod source_ops;

use cask_ops::{
    FailedInstallGuard, cask_app_dir, cask_versions, load_latest_cask_metadata_json,
    remove_cask_linked_artifacts, remove_path_if_exists, stage_cask_apps, stage_cask_binaries,
    stage_cask_linked_artifacts, with_cask_source_root, write_brew_cask_metadata,
};
pub use factory::create_installer;

use crate::core::cellar::link::Linker;
use crate::core::cellar::materialize::Cellar;
use crate::core::installer::cask::resolve_cask;
use crate::core::network::api::ApiClient;
use crate::core::network::download::{DownloadRequest, ParallelDownloader};
#[cfg(test)]
use crate::core::storage::blob::BlobCache;
use crate::core::storage::receipt::{
    InstallReceipt, InstalledKeg, find_installed, scan_installed, write_receipt,
};
use crate::core::storage::state_db::{InstalledPackage, InstalledPackageKind, StateDb};
use crate::core::storage::store::Store;

use crate::types::{Error, Formula, InstallMethod, formula_token};

const MAX_CORRUPTION_RETRIES: usize = 3;

pub struct Installer {
    api_client: ApiClient,
    downloader: ParallelDownloader,
    store: Store,
    cellar: Cellar,
    linker: Linker,
    state_db: StateDb,
    prefix: std::path::PathBuf,
}

#[derive(Debug)]
pub struct PlannedInstall {
    pub install_name: String,
    pub formula: Formula,
    pub method: InstallMethod,
}

#[derive(Debug)]
pub struct InstallPlan {
    pub items: Vec<PlannedInstall>,
}

#[derive(Debug)]
pub struct AutoInstallTargets {
    pub formulas: Vec<(String, String)>,
    pub casks: Vec<(String, String)>,
}

pub struct ExecuteResult {
    pub installed: usize,
}

impl Installer {
    fn record_install_receipt(
        &self,
        keg_path: &Path,
        install_name: &str,
        formula_name: &str,
        version: &str,
        store_key: &str,
    ) -> Result<(), Error> {
        let receipt = InstallReceipt {
            install_name: install_name.to_string(),
            formula_name: formula_name.to_string(),
            version: version.to_string(),
            store_key: store_key.to_string(),
            installed_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
        };
        write_receipt(keg_path, &receipt)?;
        self.record_installed_package(&receipt)
    }

    fn record_installed_package(&self, receipt: &InstallReceipt) -> Result<(), Error> {
        self.state_db.record_installed(&InstalledPackage {
            name: receipt.install_name.clone(),
            formula_name: receipt.formula_name.clone(),
            version: receipt.version.clone(),
            store_key: receipt.store_key.clone(),
            kind: if receipt.install_name.starts_with("cask:") {
                InstalledPackageKind::App
            } else {
                InstalledPackageKind::Formula
            },
            installed_at: receipt.installed_at,
        })
    }

    #[cfg(test)]
    pub fn new(
        api_client: ApiClient,
        blob_cache: BlobCache,
        store: Store,
        cellar: Cellar,
        linker: Linker,
        prefix: std::path::PathBuf,
    ) -> Self {
        Self {
            api_client,
            downloader: ParallelDownloader::new(blob_cache),
            store,
            cellar,
            linker,
            state_db: StateDb::in_memory().expect("test state db"),
            prefix,
        }
    }

    #[cfg(test)]
    pub async fn install(&mut self, names: &[String], link: bool) -> Result<ExecuteResult, Error> {
        let (casks, formulas): (Vec<_>, Vec<_>) = names
            .iter()
            .cloned()
            .partition(|name| name.starts_with("cask:"));

        let mut installed = 0usize;

        if !formulas.is_empty() {
            let plan = self.plan(&formulas).await?;
            installed += self.execute(plan, link).await?.installed;
        }

        if !casks.is_empty() {
            installed += self.install_casks(&casks, link).await?.installed;
        }

        Ok(ExecuteResult { installed })
    }

    pub async fn install_casks(
        &mut self,
        names: &[String],
        link: bool,
    ) -> Result<ExecuteResult, Error> {
        let mut installed = 0usize;
        for name in names {
            let token = name
                .strip_prefix("cask:")
                .expect("install_casks expects cask: prefixed names");
            self.install_single_cask(token, link).await?;
            installed += 1;
        }
        Ok(ExecuteResult { installed })
    }

    pub fn uninstall(&mut self, name: &str) -> Result<(), Error> {
        if let Some(token) = name.strip_prefix("cask:") {
            return self.uninstall_cask(token);
        }

        let installed =
            find_installed(self.cellar.root_dir(), name).ok_or(Error::NotInstalled {
                name: name.to_string(),
            })?;
        let keg_name = formula_token(&installed.name);

        let keg_path = self.cellar.keg_path(keg_name, &installed.version);
        self.linker.unlink_keg(&keg_path)?;

        self.cellar.remove_keg(keg_name, &installed.version)?;
        self.state_db.remove_installed(&installed.name)?;

        Ok(())
    }

    fn uninstall_cask(&mut self, token: &str) -> Result<(), Error> {
        let caskroom_path = self.prefix.join("Caskroom").join(token);
        if !caskroom_path.exists() {
            return Err(Error::NotInstalled {
                name: format!("cask:{token}"),
            });
        }

        if let Some(cask_json) = load_latest_cask_metadata_json(&caskroom_path, token)? {
            let cask = resolve_cask(token, &cask_json)?;
            remove_cask_linked_artifacts(&self.prefix, &cask)?;
        }

        for version_dir in cask_versions(&caskroom_path)? {
            for entry in fs::read_dir(&version_dir).map_err(|e| Error::StoreCorruption {
                message: format!(
                    "failed to read cask version directory '{}': {e}",
                    version_dir.display()
                ),
            })? {
                let path = match entry {
                    Ok(entry) => entry.path(),
                    Err(_) => continue,
                };
                if !path.is_symlink() {
                    continue;
                }
                let target = fs::read_link(&path).map_err(|e| Error::StoreCorruption {
                    message: format!("failed to read cask symlink '{}': {e}", path.display()),
                })?;
                if target.extension().and_then(|ext| ext.to_str()) == Some("app")
                    && target.starts_with(cask_app_dir(&self.prefix))
                {
                    remove_path_if_exists(&target)?;
                }
                remove_path_if_exists(&path)?;
            }
        }

        fs::remove_dir_all(&caskroom_path).map_err(|e| Error::StoreCorruption {
            message: format!(
                "failed to remove caskroom path '{}': {e}",
                caskroom_path.display()
            ),
        })?;
        self.state_db.remove_installed(&format!("cask:{token}"))?;

        Ok(())
    }

    pub fn gc(&mut self) -> Result<Vec<String>, Error> {
        let installed = scan_installed(self.cellar.root_dir())?;
        let referenced: std::collections::HashSet<String> = installed
            .into_iter()
            .map(|k| k.store_key)
            .filter(|k| !k.is_empty())
            .collect();
        let mut removed = Vec::new();
        let store_dir = self.store.root_dir().to_path_buf();
        if !store_dir.exists() {
            return Ok(removed);
        }

        for entry in fs::read_dir(&store_dir).map_err(|e| Error::StoreCorruption {
            message: format!(
                "failed to read store directory '{}': {e}",
                store_dir.display()
            ),
        })? {
            let entry = entry.map_err(|e| Error::StoreCorruption {
                message: format!("failed to read store entry: {e}"),
            })?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(store_key) = path
                .file_name()
                .and_then(|n| n.to_str())
                .map(str::to_string)
            else {
                continue;
            };
            if !referenced.contains(&store_key) {
                self.store.remove_entry(&store_key)?;
                removed.push(store_key);
            }
        }

        Ok(removed)
    }

    #[cfg(test)]
    pub fn is_installed(&self, name: &str) -> bool {
        find_installed(self.cellar.root_dir(), name).is_some()
    }

    #[cfg(test)]
    pub fn get_installed(&self, name: &str) -> Option<InstalledKeg> {
        find_installed(self.cellar.root_dir(), name)
    }

    pub fn list_installed(&self) -> Result<Vec<InstalledKeg>, Error> {
        self.state_db.list_installed().map(|packages| {
            packages
                .into_iter()
                .map(|package| InstalledKeg {
                    name: package.name,
                    version: package.version,
                    store_key: package.store_key,
                })
                .collect()
        })
    }

    async fn install_single_cask(&mut self, token: &str, link: bool) -> Result<(), Error> {
        let cask_json = self.api_client.get_cask(token).await?;
        let cask = resolve_cask(token, &cask_json)?;

        let blob_path = self
            .downloader
            .download_single(
                DownloadRequest {
                    url: cask.url.clone(),
                    sha256: cask.sha256.clone(),
                    name: cask.install_name.clone(),
                },
                None,
            )
            .await?;

        if !cask.apps.is_empty() {
            self.install_cask_apps(&cask, &cask_json, &blob_path)?;
            return Ok(());
        }

        let extracted = self.store.ensure_entry(&cask.sha256, &blob_path)?;
        let keg_path = self.cellar.keg_path(&cask.install_name, &cask.version);
        let mut cleanup = FailedInstallGuard::new(
            &self.linker,
            &self.cellar,
            &cask.install_name,
            &cask.version,
            &keg_path,
            link,
        );

        stage_cask_binaries(&extracted, &keg_path, &cask)?;

        if link {
            self.linker.link_keg(&keg_path)?;
        }

        self.record_install_receipt(
            &keg_path,
            &cask.install_name,
            &cask.install_name,
            &cask.version,
            &cask.sha256,
        )?;

        cleanup.disarm();
        Ok(())
    }

    fn install_cask_apps(
        &self,
        cask: &crate::core::installer::cask::ResolvedCask,
        cask_json: &serde_json::Value,
        blob_path: &Path,
    ) -> Result<(), Error> {
        let caskroom_path = self.prefix.join("Caskroom").join(&cask.token);
        let staged_path = caskroom_path.join(&cask.version);

        fs::create_dir_all(&staged_path).map_err(|e| Error::StoreCorruption {
            message: format!("failed to create cask staging directory: {e}"),
        })?;

        with_cask_source_root(&self.store, cask, blob_path, |source_root| {
            stage_cask_apps(source_root, &staged_path, &self.prefix, cask)?;
            stage_cask_linked_artifacts(source_root, &self.prefix, cask)
        })?;

        write_brew_cask_metadata(&caskroom_path, cask, cask_json)?;
        self.record_installed_package(&InstallReceipt {
            install_name: cask.install_name.clone(),
            formula_name: cask.install_name.clone(),
            version: cask.version.clone(),
            store_key: cask.sha256.clone(),
            installed_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
        })?;
        Ok(())
    }
}

fn dependency_cellar_path(cellar: &Cellar, installed_name: &str, version: &str) -> String {
    cellar
        .keg_path(formula_token(installed_name), version)
        .display()
        .to_string()
}

#[cfg(all(test, target_os = "macos"))]
#[path = "install/tests.rs"]
mod tests;
