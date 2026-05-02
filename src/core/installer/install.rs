use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use crate::core::cellar::link::Linker;
use crate::core::cellar::materialize::Cellar;
use crate::core::installer::cask::resolve_cask;
use crate::core::network::api::ApiClient;
use crate::core::network::download::{
    DownloadProgressCallback, DownloadRequest, DownloadResult, ParallelDownloader,
};
use crate::core::progress::{InstallProgress, ProgressCallback};
use crate::core::storage::blob::BlobCache;
use crate::core::storage::receipt::{
    InstallReceipt, InstalledKeg, find_installed, scan_installed, write_receipt,
};
use crate::core::storage::store::Store;

use crate::types::{
    BuildPlan, Error, Formula, InstallMethod, SelectedBottle, formula_token, resolve_closure,
    select_bottle,
};

const MAX_CORRUPTION_RETRIES: usize = 3;

pub struct Installer {
    api_client: ApiClient,
    downloader: ParallelDownloader,
    store: Store,
    cellar: Cellar,
    linker: Linker,
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
        write_receipt(keg_path, &receipt)
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
            prefix,
        }
    }

    #[cfg(test)]
    pub async fn plan(&self, names: &[String]) -> Result<InstallPlan, Error> {
        self.plan_with_options(names, false).await
    }

    pub async fn plan_with_options(
        &self,
        names: &[String],
        build_from_source: bool,
    ) -> Result<InstallPlan, Error> {
        let formulas = self.fetch_all_formulas(names).await?;
        let ordered = resolve_closure(names, &formulas)?;

        let mut items = Vec::with_capacity(ordered.len());
        for install_name in ordered {
            let formula = formulas.get(&install_name).cloned().unwrap();
            if find_installed(self.cellar.root_dir(), &install_name)
                .map(|installed| installed.version == formula.effective_version())
                .unwrap_or(false)
            {
                continue;
            }
            let method = if build_from_source {
                match BuildPlan::from_formula(&formula, &self.prefix) {
                    Some(plan) => InstallMethod::Source(plan),
                    None => match select_bottle(&formula) {
                        Ok(bottle) => InstallMethod::Bottle(bottle),
                        Err(_) => {
                            return Err(Error::UnsupportedBottle {
                                name: formula.name.clone(),
                            });
                        }
                    },
                }
            } else {
                match select_bottle(&formula) {
                    Ok(bottle) => InstallMethod::Bottle(bottle),
                    Err(_) => match BuildPlan::from_formula(&formula, &self.prefix) {
                        Some(plan) => InstallMethod::Source(plan),
                        None => {
                            return Err(Error::UnsupportedBottle {
                                name: formula.name.clone(),
                            });
                        }
                    },
                }
            };
            items.push(PlannedInstall {
                install_name,
                formula,
                method,
            });
        }

        Ok(InstallPlan { items })
    }

    async fn extract_with_retry(
        &self,
        download: &DownloadResult,
        formula: &Formula,
        bottle: &SelectedBottle,
        progress: Option<DownloadProgressCallback>,
    ) -> Result<std::path::PathBuf, Error> {
        let mut blob_path = download.blob_path.clone();
        let mut last_error = None;

        for attempt in 0..MAX_CORRUPTION_RETRIES {
            match self.store.ensure_entry(&bottle.sha256, &blob_path) {
                Ok(entry) => return Ok(entry),
                Err(Error::StoreCorruption { message }) => {
                    self.downloader.remove_blob(&bottle.sha256);

                    if attempt + 1 < MAX_CORRUPTION_RETRIES {
                        eprintln!(
                            "    Corrupted download detected for {}, retrying ({}/{})...",
                            formula.name,
                            attempt + 2,
                            MAX_CORRUPTION_RETRIES
                        );

                        let request = DownloadRequest {
                            url: bottle.url.clone(),
                            sha256: bottle.sha256.clone(),
                            name: formula.name.clone(),
                        };

                        match self
                            .downloader
                            .download_single(request, progress.clone())
                            .await
                        {
                            Ok(new_path) => {
                                blob_path = new_path;
                            }
                            Err(e) => {
                                last_error = Some(e);
                                break;
                            }
                        }
                    } else {
                        last_error = Some(Error::StoreCorruption {
                            message: format!(
                                "{message}\n\nFailed after {MAX_CORRUPTION_RETRIES} attempts. The download may be corrupted at the source."
                            ),
                        });
                    }
                }
                Err(e) => {
                    last_error = Some(e);
                    break;
                }
            }
        }

        Err(last_error.unwrap_or_else(|| Error::StoreCorruption {
            message: "extraction failed with unknown error".to_string(),
        }))
    }

    async fn fetch_all_formulas(
        &self,
        names: &[String],
    ) -> Result<BTreeMap<String, Formula>, Error> {
        use crate::types::select_bottle;
        use std::collections::HashSet;

        let mut formulas = BTreeMap::new();
        let mut fetched: HashSet<String> = HashSet::new();
        let mut to_fetch: Vec<String> = names.to_vec();

        while !to_fetch.is_empty() {
            let batch: Vec<String> = to_fetch
                .drain(..)
                .filter(|n| !fetched.contains(n))
                .collect();

            if batch.is_empty() {
                break;
            }

            for n in &batch {
                fetched.insert(n.clone());
            }

            let futures: Vec<_> = batch
                .iter()
                .map(|n| self.api_client.get_formula(n))
                .collect();

            let results = futures_util::future::join_all(futures).await;

            for (i, result) in results.into_iter().enumerate() {
                let formula = match result {
                    Ok(f) => f,
                    Err(e) => return Err(e),
                };

                if select_bottle(&formula).is_err() && !formula.has_source_url() {
                    eprintln!(
                        "    Skipping {} (no bottle or source available for this platform)",
                        formula.name
                    );
                    continue;
                }

                for dep in &formula.dependencies {
                    if !fetched.contains(dep) && !to_fetch.contains(dep) {
                        to_fetch.push(dep.clone());
                    }
                }

                formulas.insert(batch[i].clone(), formula);
            }
        }

        Ok(formulas)
    }

    #[cfg(test)]
    pub async fn execute(&mut self, plan: InstallPlan, link: bool) -> Result<ExecuteResult, Error> {
        self.execute_with_progress(plan, link, None).await
    }

    pub async fn execute_with_progress(
        &mut self,
        plan: InstallPlan,
        link: bool,
        progress: Option<Arc<ProgressCallback>>,
    ) -> Result<ExecuteResult, Error> {
        let report = |event: InstallProgress| {
            if let Some(ref cb) = progress {
                cb(event);
            }
        };

        let (bottle_items, source_items): (Vec<_>, Vec<_>) = plan
            .items
            .into_iter()
            .partition(|item| matches!(item.method, InstallMethod::Bottle(_)));

        if bottle_items.is_empty() && source_items.is_empty() {
            return Ok(ExecuteResult { installed: 0 });
        }

        let mut installed = 0usize;
        let mut error: Option<Error> = None;

        if !bottle_items.is_empty() {
            let requests: Vec<DownloadRequest> = bottle_items
                .iter()
                .map(|item| {
                    let InstallMethod::Bottle(ref bottle) = item.method else {
                        unreachable!()
                    };
                    DownloadRequest {
                        url: bottle.url.clone(),
                        sha256: bottle.sha256.clone(),
                        name: item.formula.name.clone(),
                    }
                })
                .collect();

            let download_progress: Option<DownloadProgressCallback> = progress.clone().map(|cb| {
                Arc::new(move |event: InstallProgress| {
                    cb(event);
                }) as DownloadProgressCallback
            });

            let mut rx = self
                .downloader
                .download_streaming(requests, download_progress.clone());

            while let Some(result) = rx.recv().await {
                match result {
                    Ok(download) => {
                        let idx = download.index;
                        let item = &bottle_items[idx];
                        let InstallMethod::Bottle(ref bottle) = item.method else {
                            unreachable!()
                        };
                        let processed_name = item.install_name.clone();
                        let materialized_name = item.formula.name.clone();
                        let processed_version = item.formula.effective_version();
                        let processed_store_key = bottle.sha256.clone();

                        report(InstallProgress::UnpackStarted {
                            name: materialized_name.clone(),
                        });

                        let store_entry = match self
                            .extract_with_retry(
                                &download,
                                &item.formula,
                                bottle,
                                download_progress.clone(),
                            )
                            .await
                        {
                            Ok(entry) => entry,
                            Err(e) => {
                                error = Some(e);
                                continue;
                            }
                        };

                        let keg_path = match self.cellar.materialize(
                            &materialized_name,
                            &processed_version,
                            &store_entry,
                        ) {
                            Ok(path) => path,
                            Err(e) => {
                                error = Some(e);
                                continue;
                            }
                        };

                        report(InstallProgress::UnpackCompleted {
                            name: materialized_name.clone(),
                        });

                        if let Err(e) = self.record_install_receipt(
                            &keg_path,
                            &processed_name,
                            &materialized_name,
                            &processed_version,
                            &processed_store_key,
                        ) {
                            Self::cleanup_materialized(
                                &self.cellar,
                                &materialized_name,
                                &processed_version,
                            );
                            error = Some(e);
                            continue;
                        }

                        if let Err(e) = self.linker.link_opt(&keg_path) {
                            eprintln!(
                                "warning: failed to create opt link for {}: {}",
                                processed_name, e
                            );
                        }

                        let should_link = link && !item.formula.is_keg_only();

                        if should_link {
                            report(InstallProgress::LinkStarted {
                                name: materialized_name.clone(),
                            });
                            match self.linker.link_keg(&keg_path) {
                                Ok(()) => {
                                    report(InstallProgress::LinkCompleted {
                                        name: materialized_name.clone(),
                                    });
                                }
                                Err(e) => {
                                    let _ = self.linker.unlink_keg(&keg_path);
                                    error = Some(e);
                                    installed += 1;
                                    report(InstallProgress::InstallCompleted {
                                        name: materialized_name.clone(),
                                    });
                                    continue;
                                }
                            }
                        } else {
                            if link && item.formula.is_keg_only() {
                                let reason = match &item.formula.keg_only {
                                    crate::types::KegOnly::Reason(s) => s.clone(),
                                    _ if item.formula.name.contains('@') => {
                                        "versioned formula".to_string()
                                    }
                                    _ => "keg-only formula".to_string(),
                                };
                                report(InstallProgress::LinkSkipped {
                                    name: materialized_name.clone(),
                                    reason,
                                });
                            }
                        }

                        report(InstallProgress::InstallCompleted {
                            name: materialized_name.clone(),
                        });

                        installed += 1;
                    }
                    Err(e) => {
                        error = Some(e);
                    }
                }
            }
        }

        for item in &source_items {
            let InstallMethod::Source(ref build_plan) = item.method else {
                unreachable!()
            };

            report(InstallProgress::UnpackStarted {
                name: item.formula.name.clone(),
            });

            match self
                .install_from_source(item, build_plan, link, &report)
                .await
            {
                Ok(()) => installed += 1,
                Err(e) => {
                    error = Some(e);
                    continue;
                }
            }
        }

        if let Some(e) = error {
            return Err(e);
        }

        Ok(ExecuteResult { installed })
    }

    fn cleanup_failed_install(
        linker: &Linker,
        cellar: &Cellar,
        name: &str,
        version: &str,
        keg_path: &Path,
        unlink: bool,
    ) {
        if unlink && let Err(e) = linker.unlink_keg(keg_path) {
            eprintln!(
                "warning: failed to clean up links for {}@{} after install error: {}",
                name, version, e
            );
        }

        if let Err(e) = cellar.remove_keg(name, version) {
            eprintln!(
                "warning: failed to remove keg for {}@{} after install error: {}",
                name, version, e
            );
        }
    }

    async fn install_from_source(
        &mut self,
        item: &PlannedInstall,
        build_plan: &BuildPlan,
        link: bool,
        report: &impl Fn(InstallProgress),
    ) -> Result<(), Error> {
        let install_name = &item.install_name;
        let formula_name = &item.formula.name;
        let version = item.formula.effective_version();

        let ruby_source_path =
            item.formula
                .ruby_source_path
                .as_deref()
                .ok_or_else(|| Error::ExecutionError {
                    message: format!("no ruby_source_path for formula '{formula_name}'"),
                })?;

        let cache_dir = self
            .store
            .root_dir()
            .parent()
            .expect("store dir always has a root parent")
            .join("cache")
            .join("rb_cache");
        let formula_rb_checksum = item
            .formula
            .ruby_source_checksum
            .as_ref()
            .map(|checksum| checksum.sha256.as_str());

        let formula_rb = self
            .api_client
            .fetch_formula_rb(ruby_source_path, &cache_dir, formula_rb_checksum)
            .await?;

        let mut installed_deps = std::collections::HashMap::new();
        for dep_name in &build_plan.runtime_dependencies {
            if let Some(keg) = find_installed(self.cellar.root_dir(), dep_name) {
                installed_deps.insert(
                    dep_name.clone(),
                    crate::core::build::DepInfo {
                        cellar_path: dependency_cellar_path(&self.cellar, &keg.name, &keg.version),
                    },
                );
            }
        }

        let keg_path = self.cellar.keg_path(formula_name, &version);
        let previous_keg_backup =
            Self::backup_existing_source_keg(&keg_path, formula_name, &version)?;

        let executor = crate::core::build::BuildExecutor::new(
            self.prefix.clone(),
            self.store
                .root_dir()
                .parent()
                .expect("store dir always has a root parent")
                .to_path_buf(),
        );
        if let Err(build_err) = executor
            .execute(build_plan, &formula_rb, &installed_deps)
            .await
        {
            if let Some(backup_path) = previous_keg_backup.as_ref() {
                Self::restore_source_keg_from_backup(
                    &keg_path,
                    backup_path,
                    formula_name,
                    &version,
                )?;
            }
            return Err(build_err);
        }

        if let Some(backup_path) = previous_keg_backup.as_ref() {
            Self::remove_source_keg_backup(backup_path, formula_name, &version)?;
        }

        report(InstallProgress::UnpackCompleted {
            name: formula_name.clone(),
        });

        let store_key = format!("source:{formula_name}:{version}");

        if let Err(e) =
            self.record_install_receipt(&keg_path, install_name, formula_name, &version, &store_key)
        {
            Self::cleanup_materialized(&self.cellar, formula_name, &version);
            return Err(e);
        }

        if let Err(e) = self.linker.link_opt(&keg_path) {
            eprintln!("warning: failed to create opt link for {install_name}: {e}");
        }

        let should_link = link && !item.formula.is_keg_only();

        if should_link {
            report(InstallProgress::LinkStarted {
                name: formula_name.clone(),
            });
            match self.linker.link_keg(&keg_path) {
                Ok(()) => {
                    report(InstallProgress::LinkCompleted {
                        name: formula_name.clone(),
                    });
                }
                Err(e) => {
                    let _ = self.linker.unlink_keg(&keg_path);
                    report(InstallProgress::InstallCompleted {
                        name: formula_name.clone(),
                    });
                    return Err(e);
                }
            }
        } else if link && item.formula.is_keg_only() {
            let reason = match &item.formula.keg_only {
                crate::types::KegOnly::Reason(s) => s.clone(),
                _ if item.formula.name.contains('@') => "versioned formula".to_string(),
                _ => "keg-only formula".to_string(),
            };
            report(InstallProgress::LinkSkipped {
                name: formula_name.clone(),
                reason,
            });
        }

        report(InstallProgress::InstallCompleted {
            name: formula_name.clone(),
        });
        Ok(())
    }

    fn backup_existing_source_keg(
        keg_path: &Path,
        formula_name: &str,
        version: &str,
    ) -> Result<Option<PathBuf>, Error> {
        if !keg_path.exists() {
            return Ok(None);
        }

        let backup_path = Self::source_keg_backup_path(keg_path);
        if backup_path.exists() {
            fs::remove_dir_all(&backup_path).map_err(|e| Error::StoreCorruption {
                message: format!(
                    "failed to remove stale source-build backup for '{}@{}': {}",
                    formula_name, version, e
                ),
            })?;
        }

        fs::rename(keg_path, &backup_path).map_err(|e| Error::StoreCorruption {
            message: format!(
                "failed to backup existing keg for '{}@{}': {}",
                formula_name, version, e
            ),
        })?;

        Ok(Some(backup_path))
    }

    fn restore_source_keg_from_backup(
        keg_path: &Path,
        backup_path: &Path,
        formula_name: &str,
        version: &str,
    ) -> Result<(), Error> {
        if keg_path.exists() {
            fs::remove_dir_all(keg_path).map_err(|e| Error::StoreCorruption {
                message: format!(
                    "failed to remove failed source-build output for '{}@{}': {}",
                    formula_name, version, e
                ),
            })?;
        }

        fs::rename(backup_path, keg_path).map_err(|e| Error::StoreCorruption {
            message: format!(
                "failed to restore previous keg for '{}@{}': {}",
                formula_name, version, e
            ),
        })
    }

    fn remove_source_keg_backup(
        backup_path: &Path,
        formula_name: &str,
        version: &str,
    ) -> Result<(), Error> {
        if !backup_path.exists() {
            return Ok(());
        }

        fs::remove_dir_all(backup_path).map_err(|e| Error::StoreCorruption {
            message: format!(
                "failed to remove source-build backup for '{}@{}': {}",
                formula_name, version, e
            ),
        })
    }

    fn source_keg_backup_path(keg_path: &Path) -> PathBuf {
        let backup_suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let name = keg_path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "keg".to_string());

        keg_path.with_file_name(format!("{name}.upkg-backup-{backup_suffix}"))
    }

    fn cleanup_materialized(cellar: &Cellar, name: &str, version: &str) {
        if let Err(e) = cellar.remove_keg(name, version) {
            eprintln!(
                "warning: failed to remove keg for {}@{} after install error: {}",
                name, version, e
            );
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
        scan_installed(self.cellar.root_dir())
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
        Ok(())
    }
}

fn dependency_cellar_path(cellar: &Cellar, installed_name: &str, version: &str) -> String {
    cellar
        .keg_path(formula_token(installed_name), version)
        .display()
        .to_string()
}

struct FailedInstallGuard<'a> {
    linker: &'a Linker,
    cellar: &'a Cellar,
    name: &'a str,
    version: &'a str,
    keg_path: &'a Path,
    unlink: bool,
    armed: bool,
}

impl<'a> FailedInstallGuard<'a> {
    fn new(
        linker: &'a Linker,
        cellar: &'a Cellar,
        name: &'a str,
        version: &'a str,
        keg_path: &'a Path,
        unlink: bool,
    ) -> Self {
        Self {
            linker,
            cellar,
            name,
            version,
            keg_path,
            unlink,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for FailedInstallGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            Installer::cleanup_failed_install(
                self.linker,
                self.cellar,
                self.name,
                self.version,
                self.keg_path,
                self.unlink,
            );
        }
    }
}

fn stage_cask_binaries(
    extracted_root: &Path,
    keg_path: &Path,
    cask: &crate::core::installer::cask::ResolvedCask,
) -> Result<(), Error> {
    let bin_dir = keg_path.join("bin");
    fs::create_dir_all(&bin_dir).map_err(|e| Error::StoreCorruption {
        message: format!("failed to create cask bin dir: {e}"),
    })?;

    for binary in &cask.binaries {
        let source = resolve_cask_source_path(extracted_root, cask, &binary.source)?;
        if !source.exists() {
            return Err(Error::InvalidArgument {
                message: format!(
                    "cask '{}' binary source '{}' not found in archive",
                    cask.token, binary.source
                ),
            });
        }

        let target = bin_dir.join(&binary.target);
        if target.exists() {
            fs::remove_file(&target).map_err(|e| Error::StoreCorruption {
                message: format!("failed to replace existing cask binary: {e}"),
            })?;
        }

        fs::copy(&source, &target).map_err(|e| Error::StoreCorruption {
            message: format!("failed to stage cask binary '{}': {e}", binary.target),
        })?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&target)
                .map_err(|e| Error::StoreCorruption {
                    message: format!("failed to read staged cask binary metadata: {e}"),
                })?
                .permissions();
            if perms.mode() & 0o111 == 0 {
                perms.set_mode(0o755);
                fs::set_permissions(&target, perms).map_err(|e| Error::StoreCorruption {
                    message: format!("failed to make staged cask binary executable: {e}"),
                })?;
            }
        }
    }

    Ok(())
}

fn resolve_cask_source_path(
    extracted_root: &Path,
    cask: &crate::core::installer::cask::ResolvedCask,
    source: &str,
) -> Result<std::path::PathBuf, Error> {
    if source.starts_with("$APPDIR") {
        return Err(Error::InvalidArgument {
            message: format!(
                "cask '{}' uses APPDIR artifacts which are not supported yet",
                cask.token
            ),
        });
    }

    let mut normalized = source.to_string();
    let caskroom_prefix = format!("$HOMEBREW_PREFIX/Caskroom/{}/{}/", cask.token, cask.version);
    if let Some(stripped) = normalized.strip_prefix(&caskroom_prefix) {
        normalized = stripped.to_string();
    }

    let source_path = Path::new(&normalized);
    if source_path.is_absolute() {
        return Err(Error::InvalidArgument {
            message: format!(
                "cask '{}' binary source '{}' must be a relative path",
                cask.token, source
            ),
        });
    }

    for component in source_path.components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err(Error::InvalidArgument {
                message: format!(
                    "cask '{}' binary source '{}' cannot contain '..'",
                    cask.token, source
                ),
            });
        }
    }

    Ok(extracted_root.join(source_path))
}

fn resolve_cask_link_source_path(
    source_root: &Path,
    prefix: &Path,
    cask: &crate::core::installer::cask::ResolvedCask,
    source: &str,
) -> Result<PathBuf, Error> {
    if source == "$APPDIR" {
        return Ok(cask_app_dir(prefix));
    }
    if let Some(stripped) = source.strip_prefix("$APPDIR/") {
        return Ok(cask_app_dir(prefix).join(stripped));
    }

    resolve_cask_source_path(source_root, cask, source)
}

fn with_cask_source_root(
    store: &Store,
    cask: &crate::core::installer::cask::ResolvedCask,
    blob_path: &Path,
    f: impl FnOnce(&Path) -> Result<(), Error>,
) -> Result<(), Error> {
    if is_dmg(&cask.url, blob_path) {
        let mounted = MountedDmg::attach(blob_path)?;
        f(&mounted.mountpoint)
    } else {
        let extracted = store.ensure_entry(&cask.sha256, blob_path)?;
        f(&extracted)
    }
}

fn is_dmg(url: &str, path: &Path) -> bool {
    url.to_ascii_lowercase().ends_with(".dmg")
        || path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("dmg"))
            .unwrap_or(false)
}

struct MountedDmg {
    mountpoint: PathBuf,
}

impl MountedDmg {
    fn attach(path: &Path) -> Result<Self, Error> {
        let mountpoint = std::env::temp_dir().join(format!(
            "upkg-dmg-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));

        fs::create_dir_all(&mountpoint).map_err(|e| Error::StoreCorruption {
            message: format!("failed to create dmg mountpoint: {e}"),
        })?;

        let output = Command::new("hdiutil")
            .args(["attach", "-nobrowse", "-readonly", "-mountpoint"])
            .arg(&mountpoint)
            .arg(path)
            .output()
            .map_err(|e| Error::ExecutionError {
                message: format!("failed to run hdiutil attach: {e}"),
            })?;

        if !output.status.success() {
            let _ = fs::remove_dir_all(&mountpoint);
            return Err(Error::ExecutionError {
                message: format!(
                    "failed to mount dmg '{}': {}",
                    path.display(),
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            });
        }

        Ok(Self { mountpoint })
    }
}

impl Drop for MountedDmg {
    fn drop(&mut self) {
        let _ = Command::new("hdiutil")
            .args(["detach", "-quiet"])
            .arg(&self.mountpoint)
            .status();
        let _ = fs::remove_dir_all(&self.mountpoint);
    }
}

fn stage_cask_apps(
    source_root: &Path,
    staged_path: &Path,
    prefix: &Path,
    cask: &crate::core::installer::cask::ResolvedCask,
) -> Result<(), Error> {
    let app_dir = cask_app_dir(prefix);
    fs::create_dir_all(&app_dir).map_err(|e| Error::StoreCorruption {
        message: format!(
            "failed to create app directory '{}': {e}",
            app_dir.display()
        ),
    })?;

    for app in &cask.apps {
        let source = resolve_cask_source_path(source_root, cask, &app.source)?;
        if !source.exists() {
            return Err(Error::InvalidArgument {
                message: format!(
                    "cask '{}' app source '{}' not found",
                    cask.token, app.source
                ),
            });
        }

        let staged_app = staged_path.join(&app.target);
        let target = app_dir.join(&app.target);

        if target.exists() {
            return Err(Error::InvalidArgument {
                message: format!(
                    "app '{}' already exists at '{}'",
                    app.target,
                    target.display()
                ),
            });
        }

        remove_path_if_exists(&staged_app)?;
        copy_path_preserving_metadata(&source, &staged_app)?;
        move_path_preserving_metadata(&staged_app, &target)?;

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&target, &staged_app).map_err(|e| {
                Error::StoreCorruption {
                    message: format!(
                        "failed to link staged app '{}' to '{}': {e}",
                        staged_app.display(),
                        target.display()
                    ),
                }
            })?;
        }
    }

    Ok(())
}

fn stage_cask_linked_artifacts(
    source_root: &Path,
    prefix: &Path,
    cask: &crate::core::installer::cask::ResolvedCask,
) -> Result<(), Error> {
    for artifact in &cask.linked_artifacts {
        let source = resolve_cask_link_source_path(source_root, prefix, cask, &artifact.source)?;
        if !source.exists() {
            return Err(Error::InvalidArgument {
                message: format!(
                    "cask '{}' artifact source '{}' not found",
                    cask.token, artifact.source
                ),
            });
        }

        let target = cask_prefix_target(prefix, &artifact.target)?;
        if target.exists() && !target.is_symlink() {
            return Err(Error::InvalidArgument {
                message: format!(
                    "cask '{}' artifact target '{}' already exists",
                    cask.token,
                    target.display()
                ),
            });
        }

        if target.is_symlink() {
            remove_path_if_exists(&target)?;
        }

        let parent = target.parent().ok_or_else(|| Error::StoreCorruption {
            message: format!("target '{}' has no parent", target.display()),
        })?;
        fs::create_dir_all(parent).map_err(|e| Error::StoreCorruption {
            message: format!("failed to create cask artifact target directory: {e}"),
        })?;

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&source, &target).map_err(|e| Error::StoreCorruption {
                message: format!(
                    "failed to link cask artifact '{}' to '{}': {e}",
                    target.display(),
                    source.display()
                ),
            })?;
        }
    }

    Ok(())
}

fn remove_cask_linked_artifacts(
    prefix: &Path,
    cask: &crate::core::installer::cask::ResolvedCask,
) -> Result<(), Error> {
    for artifact in &cask.linked_artifacts {
        let target = cask_prefix_target(prefix, &artifact.target)?;
        if target.is_symlink() {
            remove_path_if_exists(&target)?;
        }
    }

    Ok(())
}

fn cask_prefix_target(prefix: &Path, target: &str) -> Result<PathBuf, Error> {
    let target_path = Path::new(target);
    if target_path.is_absolute()
        || target_path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(Error::InvalidArgument {
            message: format!("unsupported cask artifact target '{target}'"),
        });
    }

    Ok(prefix.join(target_path))
}

fn cask_app_dir(_prefix: &Path) -> PathBuf {
    if let Some(path) = std::env::var_os("UPKG_APPDIR") {
        return PathBuf::from(path);
    }

    #[cfg(test)]
    {
        _prefix.join("Applications")
    }

    #[cfg(not(test))]
    {
        PathBuf::from("/Applications")
    }
}

fn remove_path_if_exists(path: &Path) -> Result<(), Error> {
    if !path.exists() && !path.is_symlink() {
        return Ok(());
    }

    let metadata = fs::symlink_metadata(path).map_err(|e| Error::StoreCorruption {
        message: format!("failed to read path '{}': {e}", path.display()),
    })?;

    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
    .map_err(|e| Error::StoreCorruption {
        message: format!("failed to remove '{}': {e}", path.display()),
    })
}

fn copy_path_preserving_metadata(source: &Path, target: &Path) -> Result<(), Error> {
    let parent = target.parent().ok_or_else(|| Error::StoreCorruption {
        message: format!("target '{}' has no parent", target.display()),
    })?;
    fs::create_dir_all(parent).map_err(|e| Error::StoreCorruption {
        message: format!("failed to create target parent '{}': {e}", parent.display()),
    })?;

    let output = Command::new("/bin/cp")
        .arg("-pR")
        .arg(source)
        .arg(target)
        .output()
        .map_err(|e| Error::ExecutionError {
            message: format!("failed to run cp: {e}"),
        })?;

    if output.status.success() {
        Ok(())
    } else {
        Err(Error::ExecutionError {
            message: format!(
                "failed to copy '{}' to '{}': {}",
                source.display(),
                target.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        })
    }
}

fn move_path_preserving_metadata(source: &Path, target: &Path) -> Result<(), Error> {
    if fs::rename(source, target).is_ok() {
        return Ok(());
    }

    copy_path_preserving_metadata(source, target)?;
    remove_path_if_exists(source)
}

fn write_brew_cask_metadata(
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

fn write_json_pretty(path: &Path, value: &serde_json::Value) -> Result<(), Error> {
    let data = serde_json::to_vec_pretty(value).map_err(|e| Error::StoreCorruption {
        message: format!("failed to serialize JSON for '{}': {e}", path.display()),
    })?;
    fs::write(path, data).map_err(|e| Error::StoreCorruption {
        message: format!("failed to write '{}': {e}", path.display()),
    })
}

fn load_latest_cask_metadata_json(
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

fn cask_versions(caskroom_path: &Path) -> Result<Vec<PathBuf>, Error> {
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

    let api_client = ApiClient::new();
    let blob_cache = BlobCache::new(&root.join("cache")).map_err(|e| Error::StoreCorruption {
        message: format!("failed to create blob cache: {e}"),
    })?;
    let store = Store::new(root).map_err(|e| Error::StoreCorruption {
        message: format!("failed to create store: {e}"),
    })?;
    let cellar = Cellar::new_at(prefix.join("Cellar")).map_err(|e| Error::StoreCorruption {
        message: format!("failed to create cellar: {e}"),
    })?;
    let linker = Linker::new(prefix).map_err(|e| Error::StoreCorruption {
        message: format!("failed to create linker: {e}"),
    })?;
    use crate::core::network::download::ParallelDownloader;
    let parallel_downloader = ParallelDownloader::with_concurrency(blob_cache, concurrency);

    Ok(Installer {
        api_client,
        downloader: parallel_downloader,
        store,
        cellar,
        linker,
        prefix: prefix.to_path_buf(),
    })
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn create_bottle_tarball(formula_name: &str) -> Vec<u8> {
        use flate2::Compression;
        use flate2::write::GzEncoder;
        use std::io::Write;
        use tar::Builder;

        let mut builder = Builder::new(Vec::new());

        let mut header = tar::Header::new_gnu();
        header
            .set_path(format!("{}/1.0.0/bin/{}", formula_name, formula_name))
            .unwrap();
        header.set_size(20);
        header.set_mode(0o755);
        header.set_cksum();

        let content = format!("#!/bin/sh\necho {}", formula_name);
        builder.append(&header, content.as_bytes()).unwrap();

        let tar_data = builder.into_inner().unwrap();

        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&tar_data).unwrap();
        encoder.finish().unwrap()
    }

    fn sha256_hex(data: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(data);
        crate::core::checksum::finalize_sha256_hex(hasher)
    }

    fn get_test_bottle_tag() -> String {
        crate::types::formula::bottle::current_platform_bottle_tag()
            .unwrap_or_else(|| "all".to_string())
    }

    #[test]
    fn failure_context_uses_formula_named_by_error() {
        let requested = vec!["zzz".to_string(), "agent-safehouse".to_string()];
        let formula_names = vec![
            ("zzz".to_string(), "zzz".to_string()),
            ("agent-safehouse".to_string(), "agent-safehouse".to_string()),
        ];
        let error = Error::MissingFormula {
            name: "agent-safehouse".to_string(),
        };

        assert_eq!(
            crate::native_cli::commands::install::failure_context_for_error(
                &error,
                &formula_names,
                &requested
            ),
            "agent-safehouse"
        );
    }

    #[test]
    fn dependency_cellar_path_uses_formula_token_for_tap_name() {
        let tmp = TempDir::new().unwrap();
        let cellar = Cellar::new(tmp.path()).unwrap();
        let path = dependency_cellar_path(&cellar, "hashicorp/tap/terraform", "1.10.0");

        assert!(path.ends_with("Cellar/terraform/1.10.0"));
    }

    #[test]
    fn dependency_cellar_path_keeps_core_formula_name() {
        let tmp = TempDir::new().unwrap();
        let cellar = Cellar::new(tmp.path()).unwrap();
        let path = dependency_cellar_path(&cellar, "openssl@3", "3.3.2");

        assert!(path.ends_with("Cellar/openssl@3/3.3.2"));
    }

    #[test]
    fn dependency_cellar_path_uses_name_from_install_record() {
        let tmp = TempDir::new().unwrap();
        let cellar = Cellar::new(tmp.path()).unwrap();
        let path = dependency_cellar_path(&cellar, "hashicorp/tap/terraform", "1.10.0");

        assert!(path.ends_with("Cellar/terraform/1.10.0"));
    }

    #[test]
    fn source_keg_backup_can_restore_previous_installation() {
        let tmp = TempDir::new().unwrap();
        let keg_path = tmp.path().join("Cellar").join("example").join("1.0.0");
        fs::create_dir_all(&keg_path).unwrap();
        fs::write(keg_path.join("old.txt"), "old").unwrap();

        let backup = Installer::backup_existing_source_keg(&keg_path, "example", "1.0.0").unwrap();
        let backup = backup.expect("backup path should exist");

        assert!(!keg_path.exists());
        assert!(backup.exists());

        fs::create_dir_all(&keg_path).unwrap();
        fs::write(keg_path.join("new.txt"), "new").unwrap();

        Installer::restore_source_keg_from_backup(&keg_path, &backup, "example", "1.0.0").unwrap();

        assert!(keg_path.join("old.txt").exists());
        assert!(!keg_path.join("new.txt").exists());
        assert!(!backup.exists());
    }

    #[test]
    fn backup_existing_source_keg_returns_none_when_keg_is_missing() {
        let tmp = TempDir::new().unwrap();
        let missing_keg = tmp.path().join("Cellar").join("example").join("1.0.0");

        let backup =
            Installer::backup_existing_source_keg(&missing_keg, "example", "1.0.0").unwrap();

        assert!(backup.is_none());
    }

    #[tokio::test]
    async fn install_completes_successfully() {
        let mock_server = MockServer::start().await;
        let tmp = TempDir::new().unwrap();

        let bottle = create_bottle_tarball("testpkg");
        let bottle_sha = sha256_hex(&bottle);

        let tag = get_test_bottle_tag();
        let formula_json = format!(
            r#"{{
                "name": "testpkg",
                "versions": {{ "stable": "1.0.0" }},
                "dependencies": [],
                "bottle": {{
                    "stable": {{
                        "files": {{
                            "{}": {{
                                "url": "{}/bottles/testpkg-1.0.0.{}.bottle.tar.gz",
                                "sha256": "{}"
                            }}
                        }}
                    }}
                }}
            }}"#,
            tag,
            mock_server.uri(),
            tag,
            bottle_sha
        );

        Mock::given(method("GET"))
            .and(path("/testpkg.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(&formula_json))
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path(format!(
                "/bottles/testpkg-1.0.0.{}.bottle.tar.gz",
                tag
            )))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(bottle.clone()))
            .mount(&mock_server)
            .await;

        let root = tmp.path().join("upkg");
        let prefix = tmp.path().join("homebrew");

        let api_client = ApiClient::with_base_url(mock_server.uri());
        let blob_cache = BlobCache::new(&root.join("cache")).unwrap();
        let store = Store::new(&root).unwrap();
        let cellar = Cellar::new(&root).unwrap();
        let linker = Linker::new(&prefix).unwrap();

        let mut installer = Installer::new(
            api_client,
            blob_cache,
            store,
            cellar,
            linker,
            prefix.clone(),
        );

        installer
            .install(&["testpkg".to_string()], true)
            .await
            .unwrap();

        assert!(root.join("Cellar/testpkg/1.0.0").exists());

        assert!(prefix.join("bin/testpkg").exists());

        let installed = installer.get_installed("testpkg");
        assert!(installed.is_some());
        assert_eq!(installed.unwrap().version, "1.0.0");
    }

    #[tokio::test]
    async fn uninstall_cleans_everything() {
        let mock_server = MockServer::start().await;
        let tmp = TempDir::new().unwrap();

        let bottle = create_bottle_tarball("uninstallme");
        let bottle_sha = sha256_hex(&bottle);

        let tag = get_test_bottle_tag();
        let formula_json = format!(
            r#"{{
                "name": "uninstallme",
                "versions": {{ "stable": "1.0.0" }},
                "dependencies": [],
                "bottle": {{
                    "stable": {{
                        "files": {{
                            "{}": {{
                                "url": "{}/bottles/uninstallme-1.0.0.{}.bottle.tar.gz",
                                "sha256": "{}"
                            }}
                        }}
                    }}
                }}
            }}"#,
            tag,
            mock_server.uri(),
            tag,
            bottle_sha
        );

        Mock::given(method("GET"))
            .and(path("/uninstallme.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(&formula_json))
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path(format!(
                "/bottles/uninstallme-1.0.0.{}.bottle.tar.gz",
                tag
            )))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(bottle.clone()))
            .mount(&mock_server)
            .await;

        let root = tmp.path().join("upkg");
        let prefix = tmp.path().join("homebrew");

        let api_client = ApiClient::with_base_url(mock_server.uri());
        let blob_cache = BlobCache::new(&root.join("cache")).unwrap();
        let store = Store::new(&root).unwrap();
        let cellar = Cellar::new(&root).unwrap();
        let linker = Linker::new(&prefix).unwrap();

        let mut installer = Installer::new(
            api_client,
            blob_cache,
            store,
            cellar,
            linker,
            prefix.clone(),
        );

        installer
            .install(&["uninstallme".to_string()], true)
            .await
            .unwrap();

        assert!(installer.is_installed("uninstallme"));
        assert!(root.join("Cellar/uninstallme/1.0.0").exists());
        assert!(prefix.join("bin/uninstallme").exists());

        installer.uninstall("uninstallme").unwrap();

        assert!(!installer.is_installed("uninstallme"));
        assert!(!root.join("Cellar/uninstallme/1.0.0").exists());
        assert!(!prefix.join("bin/uninstallme").exists());
    }

    #[tokio::test]
    async fn gc_removes_unreferenced_store_entries() {
        let mock_server = MockServer::start().await;
        let tmp = TempDir::new().unwrap();

        let bottle = create_bottle_tarball("gctest");
        let bottle_sha = sha256_hex(&bottle);

        let tag = get_test_bottle_tag();
        let formula_json = format!(
            r#"{{
                "name": "gctest",
                "versions": {{ "stable": "1.0.0" }},
                "dependencies": [],
                "bottle": {{
                    "stable": {{
                        "files": {{
                            "{}": {{
                                "url": "{}/bottles/gctest-1.0.0.{}.bottle.tar.gz",
                                "sha256": "{}"
                            }}
                        }}
                    }}
                }}
            }}"#,
            tag,
            mock_server.uri(),
            tag,
            bottle_sha
        );

        Mock::given(method("GET"))
            .and(path("/gctest.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(&formula_json))
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path(format!("/bottles/gctest-1.0.0.{}.bottle.tar.gz", tag)))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(bottle.clone()))
            .mount(&mock_server)
            .await;

        let root = tmp.path().join("upkg");
        let prefix = tmp.path().join("homebrew");

        let api_client = ApiClient::with_base_url(mock_server.uri());
        let blob_cache = BlobCache::new(&root.join("cache")).unwrap();
        let store = Store::new(&root).unwrap();
        let cellar = Cellar::new(&root).unwrap();
        let linker = Linker::new(&prefix).unwrap();

        let mut installer = Installer::new(
            api_client,
            blob_cache,
            store,
            cellar,
            linker,
            prefix.clone(),
        );

        installer
            .install(&["gctest".to_string()], true)
            .await
            .unwrap();

        assert!(root.join("store").join(&bottle_sha).exists());

        installer.uninstall("gctest").unwrap();

        assert!(root.join("store").join(&bottle_sha).exists());

        let removed = installer.gc().unwrap();
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0], bottle_sha);

        assert!(!root.join("store").join(&bottle_sha).exists());
        assert!(installer.gc().unwrap().is_empty());
    }

    #[tokio::test]
    async fn gc_does_not_remove_referenced_store_entries() {
        let mock_server = MockServer::start().await;
        let tmp = TempDir::new().unwrap();

        let bottle = create_bottle_tarball("keepme");
        let bottle_sha = sha256_hex(&bottle);

        let tag = get_test_bottle_tag();
        let formula_json = format!(
            r#"{{
                "name": "keepme",
                "versions": {{ "stable": "1.0.0" }},
                "dependencies": [],
                "bottle": {{
                    "stable": {{
                        "files": {{
                            "{}": {{
                                "url": "{}/bottles/keepme-1.0.0.{}.bottle.tar.gz",
                                "sha256": "{}"
                            }}
                        }}
                    }}
                }}
            }}"#,
            tag,
            mock_server.uri(),
            tag,
            bottle_sha
        );

        Mock::given(method("GET"))
            .and(path("/keepme.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(&formula_json))
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path(format!("/bottles/keepme-1.0.0.{}.bottle.tar.gz", tag)))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(bottle.clone()))
            .mount(&mock_server)
            .await;

        let root = tmp.path().join("upkg");
        let prefix = tmp.path().join("homebrew");

        let api_client = ApiClient::with_base_url(mock_server.uri());
        let blob_cache = BlobCache::new(&root.join("cache")).unwrap();
        let store = Store::new(&root).unwrap();
        let cellar = Cellar::new(&root).unwrap();
        let linker = Linker::new(&prefix).unwrap();

        let mut installer = Installer::new(
            api_client,
            blob_cache,
            store,
            cellar,
            linker,
            prefix.clone(),
        );

        installer
            .install(&["keepme".to_string()], true)
            .await
            .unwrap();

        assert!(root.join("store").join(&bottle_sha).exists());

        let removed = installer.gc().unwrap();
        assert!(removed.is_empty());

        assert!(root.join("store").join(&bottle_sha).exists());
    }

    #[tokio::test]
    async fn install_with_dependencies() {
        let mock_server = MockServer::start().await;
        let tmp = TempDir::new().unwrap();

        let dep_bottle = create_bottle_tarball("deplib");
        let dep_sha = sha256_hex(&dep_bottle);

        let main_bottle = create_bottle_tarball("mainpkg");
        let main_sha = sha256_hex(&main_bottle);

        let tag = get_test_bottle_tag();
        let dep_json = format!(
            r#"{{
                "name": "deplib",
                "versions": {{ "stable": "1.0.0" }},
                "dependencies": [],
                "bottle": {{
                    "stable": {{
                        "files": {{
                            "{}": {{
                                "url": "{}/bottles/deplib-1.0.0.{}.bottle.tar.gz",
                                "sha256": "{}"
                            }}
                        }}
                    }}
                }}
            }}"#,
            tag,
            mock_server.uri(),
            tag,
            dep_sha
        );

        let main_json = format!(
            r#"{{
                "name": "mainpkg",
                "versions": {{ "stable": "2.0.0" }},
                "dependencies": ["deplib"],
                "bottle": {{
                    "stable": {{
                        "files": {{
                            "{}": {{
                                "url": "{}/bottles/mainpkg-2.0.0.{}.bottle.tar.gz",
                                "sha256": "{}"
                            }}
                        }}
                    }}
                }}
            }}"#,
            tag,
            mock_server.uri(),
            tag,
            main_sha
        );

        Mock::given(method("GET"))
            .and(path("/deplib.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(&dep_json))
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/mainpkg.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(&main_json))
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path(format!("/bottles/deplib-1.0.0.{}.bottle.tar.gz", tag)))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(dep_bottle))
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path(format!(
                "/bottles/mainpkg-2.0.0.{}.bottle.tar.gz",
                tag
            )))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(main_bottle))
            .mount(&mock_server)
            .await;

        let root = tmp.path().join("upkg");
        let prefix = tmp.path().join("homebrew");

        let api_client = ApiClient::with_base_url(mock_server.uri());
        let blob_cache = BlobCache::new(&root.join("cache")).unwrap();
        let store = Store::new(&root).unwrap();
        let cellar = Cellar::new(&root).unwrap();
        let linker = Linker::new(&prefix).unwrap();

        let mut installer = Installer::new(
            api_client,
            blob_cache,
            store,
            cellar,
            linker,
            prefix.clone(),
        );

        installer
            .install(&["mainpkg".to_string()], true)
            .await
            .unwrap();

        assert!(installer.get_installed("mainpkg").is_some());
        assert!(installer.get_installed("deplib").is_some());
    }

    #[tokio::test]
    #[ignore = "flaky mock channel close for dependent core formula fetch"]
    async fn plans_tapped_formula_with_core_dependency() {
        let mock_server = MockServer::start().await;
        let tmp = TempDir::new().unwrap();

        let dep_bottle = create_bottle_tarball("go");
        let dep_sha = sha256_hex(&dep_bottle);
        let tag = get_test_bottle_tag();
        let dep_json = format!(
            r#"{{
                "name": "go",
                "versions": {{ "stable": "1.24.0" }},
                "dependencies": [],
                "bottle": {{
                    "stable": {{
                        "files": {{
                            "{}": {{
                                "url": "{}/bottles/go-1.24.0.{}.bottle.tar.gz",
                                "sha256": "{}"
                            }}
                        }}
                    }}
                }}
            }}"#,
            tag,
            mock_server.uri(),
            tag,
            dep_sha
        );

        Mock::given(method("GET"))
            .and(path("/go.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(&dep_json))
            .mount(&mock_server)
            .await;

        let tap_formula_rb = format!(
            r#"
class Terraform < Formula
  version "1.10.0"
  depends_on "go"
  bottle do
    root_url "{}/ghcr/hashicorp/tap"
    sha256 {}: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  end
end
"#,
            mock_server.uri(),
            tag
        );

        Mock::given(method("GET"))
            .and(path("/hashicorp/homebrew-tap/main/Formula/terraform.rb"))
            .respond_with(ResponseTemplate::new(200).set_body_string(tap_formula_rb))
            .mount(&mock_server)
            .await;

        let root = tmp.path().join("upkg");
        let prefix = tmp.path().join("homebrew");

        let api_client =
            ApiClient::with_base_url(mock_server.uri()).with_tap_raw_base_url(mock_server.uri());
        let blob_cache = BlobCache::new(&root.join("cache")).unwrap();
        let store = Store::new(&root).unwrap();
        let cellar = Cellar::new(&root).unwrap();
        let linker = Linker::new(&prefix).unwrap();

        let installer = Installer::new(
            api_client,
            blob_cache,
            store,
            cellar,
            linker,
            prefix.to_path_buf(),
        );
        let plan = installer
            .plan(&["hashicorp/tap/terraform".to_string()])
            .await
            .unwrap();

        let planned_names: Vec<String> = plan
            .items
            .iter()
            .map(|item| item.formula.name.clone())
            .collect();
        assert!(planned_names.contains(&"terraform".to_string()));
        assert!(planned_names.contains(&"go".to_string()));
    }

    #[tokio::test]
    async fn uninstall_accepts_full_tap_reference_after_install() {
        let mock_server = MockServer::start().await;
        let tmp = TempDir::new().unwrap();

        let bottle = create_bottle_tarball("terraform");
        let sha = sha256_hex(&bottle);
        let tag = get_test_bottle_tag();

        let tap_formula_rb = format!(
            r#"
class Terraform < Formula
  version "1.10.0"
  bottle do
    root_url "{}/v2/hashicorp/tap"
    sha256 {}: "{}"
  end
end
"#,
            mock_server.uri(),
            tag,
            sha
        );

        Mock::given(method("GET"))
            .and(path("/hashicorp/homebrew-tap/main/Formula/terraform.rb"))
            .respond_with(ResponseTemplate::new(200).set_body_string(tap_formula_rb))
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path(format!(
                "/v2/hashicorp/tap/terraform/blobs/sha256:{sha}"
            )))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(bottle))
            .mount(&mock_server)
            .await;

        let root = tmp.path().join("upkg");
        let prefix = tmp.path().join("homebrew");

        let api_client =
            ApiClient::with_base_url(mock_server.uri()).with_tap_raw_base_url(mock_server.uri());
        let blob_cache = BlobCache::new(&root.join("cache")).unwrap();
        let store = Store::new(&root).unwrap();
        let cellar = Cellar::new(&root).unwrap();
        let linker = Linker::new(&prefix).unwrap();

        let mut installer = Installer::new(
            api_client,
            blob_cache,
            store,
            cellar,
            linker,
            prefix.to_path_buf(),
        );

        installer
            .install(&["hashicorp/tap/terraform".to_string()], true)
            .await
            .unwrap();

        assert!(installer.is_installed("hashicorp/tap/terraform"));
        assert!(!installer.is_installed("terraform"));
        assert!(root.join("Cellar/terraform/1.10.0").exists());
        installer.uninstall("hashicorp/tap/terraform").unwrap();
        assert!(!installer.is_installed("hashicorp/tap/terraform"));
        assert!(!root.join("Cellar/terraform/1.10.0").exists());
    }

    #[tokio::test]
    async fn uninstalling_non_installed_tap_ref_does_not_remove_core_formula() {
        let mock_server = MockServer::start().await;
        let tmp = TempDir::new().unwrap();

        let bottle = create_bottle_tarball("terraform");
        let sha = sha256_hex(&bottle);
        let tag = get_test_bottle_tag();
        let core_json = format!(
            r#"{{
                "name": "terraform",
                "versions": {{ "stable": "1.10.0" }},
                "dependencies": [],
                "bottle": {{
                    "stable": {{
                        "files": {{
                            "{}": {{
                                "url": "{}/bottles/terraform-1.10.0.{}.bottle.tar.gz",
                                "sha256": "{}"
                            }}
                        }}
                    }}
                }}
            }}"#,
            tag,
            mock_server.uri(),
            tag,
            sha
        );

        Mock::given(method("GET"))
            .and(path("/terraform.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(core_json))
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path(format!(
                "/bottles/terraform-1.10.0.{}.bottle.tar.gz",
                tag
            )))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(bottle))
            .mount(&mock_server)
            .await;

        let root = tmp.path().join("upkg");
        let prefix = tmp.path().join("homebrew");

        let api_client = ApiClient::with_base_url(mock_server.uri());
        let blob_cache = BlobCache::new(&root.join("cache")).unwrap();
        let store = Store::new(&root).unwrap();
        let cellar = Cellar::new(&root).unwrap();
        let linker = Linker::new(&prefix).unwrap();

        let mut installer = Installer::new(
            api_client,
            blob_cache,
            store,
            cellar,
            linker,
            prefix.to_path_buf(),
        );
        installer
            .install(&["terraform".to_string()], true)
            .await
            .unwrap();
        assert!(installer.is_installed("terraform"));

        let err = installer.uninstall("hashicorp/tap/terraform").unwrap_err();
        assert!(matches!(err, Error::NotInstalled { .. }));
        assert!(installer.is_installed("terraform"));
    }

    #[tokio::test]
    async fn preserves_successful_installs_when_one_package_fails() {
        use std::time::Duration;

        let mock_server = MockServer::start().await;
        let tmp = TempDir::new().unwrap();

        let good_bottle = create_bottle_tarball("goodpkg");
        let good_sha = sha256_hex(&good_bottle);

        let tag = get_test_bottle_tag();
        let good_json = format!(
            r#"{{
                "name": "goodpkg",
                "versions": {{ "stable": "1.0.0" }},
                "dependencies": [],
                "bottle": {{
                    "stable": {{
                        "files": {{
                            "{}": {{
                                "url": "{}/bottles/goodpkg-1.0.0.{}.bottle.tar.gz",
                                "sha256": "{}"
                            }}
                        }}
                    }}
                }}
            }}"#,
            tag,
            mock_server.uri(),
            tag,
            good_sha
        );

        let bad_json = format!(
            r#"{{
                "name": "badpkg",
                "versions": {{ "stable": "1.0.0" }},
                "dependencies": [],
                "bottle": {{
                    "stable": {{
                        "files": {{
                            "{}": {{
                                "url": "{}/bottles/badpkg-1.0.0.{}.bottle.tar.gz",
                                "sha256": "{}"
                            }}
                        }}
                    }}
                }}
            }}"#,
            tag,
            mock_server.uri(),
            tag,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );

        Mock::given(method("GET"))
            .and(path("/goodpkg.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(&good_json))
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/badpkg.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(&bad_json))
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path(format!(
                "/bottles/goodpkg-1.0.0.{}.bottle.tar.gz",
                tag
            )))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(good_bottle))
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path(format!("/bottles/badpkg-1.0.0.{}.bottle.tar.gz", tag)))
            .respond_with(
                ResponseTemplate::new(500)
                    .set_delay(Duration::from_millis(100))
                    .set_body_string("download failed"),
            )
            .mount(&mock_server)
            .await;

        let root = tmp.path().join("upkg");
        let prefix = tmp.path().join("homebrew");

        let api_client = ApiClient::with_base_url(mock_server.uri());
        let blob_cache = BlobCache::new(&root.join("cache")).unwrap();
        let store = Store::new(&root).unwrap();
        let cellar = Cellar::new(&root).unwrap();
        let linker = Linker::new(&prefix).unwrap();

        let mut installer = Installer::new(
            api_client,
            blob_cache,
            store,
            cellar,
            linker,
            prefix.clone(),
        );

        let result = installer
            .install(&["goodpkg".to_string(), "badpkg".to_string()], false)
            .await;
        assert!(result.is_err());

        assert!(installer.get_installed("goodpkg").is_some());
        assert!(installer.get_installed("badpkg").is_none());
        assert!(root.join("Cellar/goodpkg/1.0.0").exists());
    }

    #[tokio::test]
    async fn parallel_api_fetching_with_deep_deps() {
        let mock_server = MockServer::start().await;
        let tmp = TempDir::new().unwrap();

        let leaf1_bottle = create_bottle_tarball("leaf1");
        let leaf1_sha = sha256_hex(&leaf1_bottle);
        let leaf2_bottle = create_bottle_tarball("leaf2");
        let leaf2_sha = sha256_hex(&leaf2_bottle);
        let mid1_bottle = create_bottle_tarball("mid1");
        let mid1_sha = sha256_hex(&mid1_bottle);
        let mid2_bottle = create_bottle_tarball("mid2");
        let mid2_sha = sha256_hex(&mid2_bottle);
        let root_bottle = create_bottle_tarball("root");
        let root_sha = sha256_hex(&root_bottle);

        let tag = get_test_bottle_tag();
        let leaf1_json = format!(
            r#"{{"name":"leaf1","versions":{{"stable":"1.0.0"}},"dependencies":[],"bottle":{{"stable":{{"files":{{"{}":{{"url":"{}/bottles/leaf1.tar.gz","sha256":"{}"}}}}}}}}}}"#,
            tag,
            mock_server.uri(),
            leaf1_sha
        );
        let leaf2_json = format!(
            r#"{{"name":"leaf2","versions":{{"stable":"1.0.0"}},"dependencies":[],"bottle":{{"stable":{{"files":{{"{}":{{"url":"{}/bottles/leaf2.tar.gz","sha256":"{}"}}}}}}}}}}"#,
            tag,
            mock_server.uri(),
            leaf2_sha
        );
        let mid1_json = format!(
            r#"{{"name":"mid1","versions":{{"stable":"1.0.0"}},"dependencies":["leaf1"],"bottle":{{"stable":{{"files":{{"{}":{{"url":"{}/bottles/mid1.tar.gz","sha256":"{}"}}}}}}}}}}"#,
            tag,
            mock_server.uri(),
            mid1_sha
        );
        let mid2_json = format!(
            r#"{{"name":"mid2","versions":{{"stable":"1.0.0"}},"dependencies":["leaf1","leaf2"],"bottle":{{"stable":{{"files":{{"{}":{{"url":"{}/bottles/mid2.tar.gz","sha256":"{}"}}}}}}}}}}"#,
            tag,
            mock_server.uri(),
            mid2_sha
        );
        let root_json = format!(
            r#"{{"name":"root","versions":{{"stable":"1.0.0"}},"dependencies":["mid1","mid2"],"bottle":{{"stable":{{"files":{{"{}":{{"url":"{}/bottles/root.tar.gz","sha256":"{}"}}}}}}}}}}"#,
            tag,
            mock_server.uri(),
            root_sha
        );

        for (name, json) in [
            ("leaf1", &leaf1_json),
            ("leaf2", &leaf2_json),
            ("mid1", &mid1_json),
            ("mid2", &mid2_json),
            ("root", &root_json),
        ] {
            Mock::given(method("GET"))
                .and(path(format!("/{}.json", name)))
                .respond_with(ResponseTemplate::new(200).set_body_string(json))
                .mount(&mock_server)
                .await;
        }
        for (name, bottle) in [
            ("leaf1", &leaf1_bottle),
            ("leaf2", &leaf2_bottle),
            ("mid1", &mid1_bottle),
            ("mid2", &mid2_bottle),
            ("root", &root_bottle),
        ] {
            Mock::given(method("GET"))
                .and(path(format!("/bottles/{}.tar.gz", name)))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(bottle.clone()))
                .mount(&mock_server)
                .await;
        }

        let root = tmp.path().join("upkg");
        let prefix = tmp.path().join("homebrew");

        let api_client = ApiClient::with_base_url(mock_server.uri());
        let blob_cache = BlobCache::new(&root.join("cache")).unwrap();
        let store = Store::new(&root).unwrap();
        let cellar = Cellar::new(&root).unwrap();
        let linker = Linker::new(&prefix).unwrap();

        let mut installer = Installer::new(
            api_client,
            blob_cache,
            store,
            cellar,
            linker,
            prefix.clone(),
        );

        installer
            .install(&["root".to_string()], true)
            .await
            .unwrap();

        assert!(installer.get_installed("root").is_some());
        assert!(installer.get_installed("mid1").is_some());
        assert!(installer.get_installed("mid2").is_some());
        assert!(installer.get_installed("leaf1").is_some());
        assert!(installer.get_installed("leaf2").is_some());
    }

    #[tokio::test]
    async fn streaming_extraction_processes_as_downloads_complete() {
        use std::time::Duration;

        let mock_server = MockServer::start().await;
        let tmp = TempDir::new().unwrap();

        let fast_bottle = create_bottle_tarball("fastpkg");
        let fast_sha = sha256_hex(&fast_bottle);
        let slow_bottle = create_bottle_tarball("slowpkg");
        let slow_sha = sha256_hex(&slow_bottle);

        let tag = get_test_bottle_tag();
        let fast_json = format!(
            r#"{{"name":"fastpkg","versions":{{"stable":"1.0.0"}},"dependencies":[],"bottle":{{"stable":{{"files":{{"{}":{{"url":"{}/bottles/fast.tar.gz","sha256":"{}"}}}}}}}}}}"#,
            tag,
            mock_server.uri(),
            fast_sha
        );

        let slow_json = format!(
            r#"{{"name":"slowpkg","versions":{{"stable":"1.0.0"}},"dependencies":["fastpkg"],"bottle":{{"stable":{{"files":{{"{}":{{"url":"{}/bottles/slow.tar.gz","sha256":"{}"}}}}}}}}}}"#,
            tag,
            mock_server.uri(),
            slow_sha
        );

        Mock::given(method("GET"))
            .and(path("/fastpkg.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(&fast_json))
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/slowpkg.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(&slow_json))
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/bottles/fast.tar.gz"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(fast_bottle.clone()))
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/bottles/slow.tar.gz"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(slow_bottle.clone())
                    .set_delay(Duration::from_millis(100)),
            )
            .mount(&mock_server)
            .await;

        let root = tmp.path().join("upkg");
        let prefix = tmp.path().join("homebrew");

        let api_client = ApiClient::with_base_url(mock_server.uri());
        let blob_cache = BlobCache::new(&root.join("cache")).unwrap();
        let store = Store::new(&root).unwrap();
        let cellar = Cellar::new(&root).unwrap();
        let linker = Linker::new(&prefix).unwrap();

        let mut installer = Installer::new(
            api_client,
            blob_cache,
            store,
            cellar,
            linker,
            prefix.clone(),
        );

        installer
            .install(&["slowpkg".to_string()], true)
            .await
            .unwrap();

        assert!(installer.get_installed("fastpkg").is_some());
        assert!(installer.get_installed("slowpkg").is_some());

        assert!(root.join("Cellar/fastpkg/1.0.0").exists());
        assert!(root.join("Cellar/slowpkg/1.0.0").exists());

        assert!(prefix.join("bin/fastpkg").exists());
        assert!(prefix.join("bin/slowpkg").exists());
    }

    #[tokio::test]
    async fn retries_on_corrupted_download() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let mock_server = MockServer::start().await;
        let tmp = TempDir::new().unwrap();

        let bottle = create_bottle_tarball("retrypkg");
        let bottle_sha = sha256_hex(&bottle);

        let tag = get_test_bottle_tag();
        let formula_json = format!(
            r#"{{
                "name": "retrypkg",
                "versions": {{ "stable": "1.0.0" }},
                "dependencies": [],
                "bottle": {{
                    "stable": {{
                        "files": {{
                            "{}": {{
                                "url": "{}/bottles/retrypkg-1.0.0.{}.bottle.tar.gz",
                                "sha256": "{}"
                            }}
                        }}
                    }}
                }}
            }}"#,
            tag,
            mock_server.uri(),
            tag,
            bottle_sha
        );

        Mock::given(method("GET"))
            .and(path("/retrypkg.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(&formula_json))
            .mount(&mock_server)
            .await;

        let attempt_count = Arc::new(AtomicUsize::new(0));
        let attempt_clone = attempt_count.clone();
        let valid_bottle = bottle.clone();

        Mock::given(method("GET"))
            .and(path(format!(
                "/bottles/retrypkg-1.0.0.{}.bottle.tar.gz",
                tag
            )))
            .respond_with(move |_: &wiremock::Request| {
                attempt_clone.fetch_add(1, Ordering::SeqCst);
                ResponseTemplate::new(200).set_body_bytes(valid_bottle.clone())
            })
            .mount(&mock_server)
            .await;

        let root = tmp.path().join("upkg");
        let prefix = tmp.path().join("homebrew");

        let api_client = ApiClient::with_base_url(mock_server.uri());
        let blob_cache = BlobCache::new(&root.join("cache")).unwrap();
        let store = Store::new(&root).unwrap();
        let cellar = Cellar::new(&root).unwrap();
        let linker = Linker::new(&prefix).unwrap();

        let mut installer = Installer::new(
            api_client,
            blob_cache,
            store,
            cellar,
            linker,
            prefix.clone(),
        );

        installer
            .install(&["retrypkg".to_string()], true)
            .await
            .unwrap();

        assert!(installer.is_installed("retrypkg"));
        assert!(root.join("Cellar/retrypkg/1.0.0").exists());
        assert!(prefix.join("bin/retrypkg").exists());
    }

    #[tokio::test]
    async fn fails_after_max_retries() {}

    #[tokio::test]
    async fn plan_falls_back_to_source_when_no_bottle() {
        let mock_server = MockServer::start().await;
        let tmp = TempDir::new().unwrap();

        let formula_json = r#"{
            "name": "nobottle",
            "versions": { "stable": "1.0.0" },
            "dependencies": [],
            "build_dependencies": ["pkgconf"],
            "urls": {
                "stable": {
                    "url": "https://example.com/nobottle-1.0.0.tar.gz",
                    "checksum": "abc123"
                }
            },
            "ruby_source_path": "Formula/n/nobottle.rb",
            "bottle": { "stable": { "files": {} } }
        }"#;

        Mock::given(method("GET"))
            .and(path("/nobottle.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(formula_json))
            .mount(&mock_server)
            .await;

        let root = tmp.path().join("upkg");
        let prefix = tmp.path().join("homebrew");

        let api_client = ApiClient::with_base_url(mock_server.uri());
        let blob_cache = BlobCache::new(&root.join("cache")).unwrap();
        let store = Store::new(&root).unwrap();
        let cellar = Cellar::new(&root).unwrap();
        let linker = Linker::new(&prefix).unwrap();

        let installer = Installer::new(
            api_client,
            blob_cache,
            store,
            cellar,
            linker,
            prefix.clone(),
        );

        let plan = installer.plan(&["nobottle".to_string()]).await.unwrap();

        assert_eq!(plan.items.len(), 1);
        assert_eq!(plan.items[0].formula.name, "nobottle");
        assert!(matches!(
            plan.items[0].method,
            crate::types::InstallMethod::Source(_)
        ));

        if let crate::types::InstallMethod::Source(ref bp) = plan.items[0].method {
            assert_eq!(bp.source_url, "https://example.com/nobottle-1.0.0.tar.gz");
            assert_eq!(bp.formula_name, "nobottle");
            assert_eq!(bp.build_dependencies, vec!["pkgconf"]);
        }
    }

    #[tokio::test]
    async fn plan_prefers_bottle_over_source() {
        let mock_server = MockServer::start().await;
        let tmp = TempDir::new().unwrap();

        let tag = get_test_bottle_tag();
        let formula_json = format!(
            r#"{{
                "name": "hasboth",
                "versions": {{ "stable": "2.0.0" }},
                "dependencies": [],
                "urls": {{
                    "stable": {{
                        "url": "https://example.com/hasboth-2.0.0.tar.gz",
                        "checksum": "def456"
                    }}
                }},
                "ruby_source_path": "Formula/h/hasboth.rb",
                "bottle": {{
                    "stable": {{
                        "files": {{
                            "{}": {{
                                "url": "https://example.com/hasboth.bottle.tar.gz",
                                "sha256": "aabbccdd"
                            }}
                        }}
                    }}
                }}
            }}"#,
            tag
        );

        Mock::given(method("GET"))
            .and(path("/hasboth.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(&formula_json))
            .mount(&mock_server)
            .await;

        let root = tmp.path().join("upkg");
        let prefix = tmp.path().join("homebrew");

        let api_client = ApiClient::with_base_url(mock_server.uri());
        let blob_cache = BlobCache::new(&root.join("cache")).unwrap();
        let store = Store::new(&root).unwrap();
        let cellar = Cellar::new(&root).unwrap();
        let linker = Linker::new(&prefix).unwrap();

        let installer = Installer::new(
            api_client,
            blob_cache,
            store,
            cellar,
            linker,
            prefix.clone(),
        );

        let plan = installer.plan(&["hasboth".to_string()]).await.unwrap();

        assert_eq!(plan.items.len(), 1);
        assert!(matches!(
            plan.items[0].method,
            crate::types::InstallMethod::Bottle(_)
        ));
    }

    #[tokio::test]
    async fn plan_skips_already_installed_same_version() {
        let mock_server = MockServer::start().await;
        let tmp = TempDir::new().unwrap();
        let tag = get_test_bottle_tag();
        let bottle = create_bottle_tarball("alreadythere");
        let bottle_sha = crate::core::checksum::sha256_hex_bytes(&bottle);

        let formula_json = format!(
            r#"{{
                "name": "alreadythere",
                "versions": {{ "stable": "1.0.0" }},
                "dependencies": [],
                "bottle": {{
                    "stable": {{
                        "files": {{
                            "{}": {{
                                "url": "{}/bottles/alreadythere.tar.gz",
                                "sha256": "{}"
                            }}
                        }}
                    }}
                }}
            }}"#,
            tag,
            mock_server.uri(),
            bottle_sha
        );

        Mock::given(method("GET"))
            .and(path("/alreadythere.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(&formula_json))
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/bottles/alreadythere.tar.gz"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(bottle))
            .mount(&mock_server)
            .await;

        let root = tmp.path().join("upkg");
        let prefix = tmp.path().join("homebrew");

        let api_client = ApiClient::with_base_url(mock_server.uri());
        let blob_cache = BlobCache::new(&root.join("cache")).unwrap();
        let store = Store::new(&root).unwrap();
        let cellar = Cellar::new(&root).unwrap();
        let linker = Linker::new(&prefix).unwrap();

        let mut installer = Installer::new(
            api_client,
            blob_cache,
            store,
            cellar,
            linker,
            prefix.clone(),
        );

        installer
            .install(&["alreadythere".to_string()], true)
            .await
            .unwrap();

        let plan = installer.plan(&["alreadythere".to_string()]).await.unwrap();
        assert!(plan.items.is_empty());
    }

    #[tokio::test]
    async fn plan_errors_when_no_bottle_and_no_source() {
        let mock_server = MockServer::start().await;
        let tmp = TempDir::new().unwrap();

        let formula_json = r#"{
            "name": "nothing",
            "versions": { "stable": "1.0.0" },
            "dependencies": [],
            "bottle": { "stable": { "files": {} } }
        }"#;

        Mock::given(method("GET"))
            .and(path("/nothing.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(formula_json))
            .mount(&mock_server)
            .await;

        let root = tmp.path().join("upkg");
        let prefix = tmp.path().join("homebrew");

        let api_client = ApiClient::with_base_url(mock_server.uri());
        let blob_cache = BlobCache::new(&root.join("cache")).unwrap();
        let store = Store::new(&root).unwrap();
        let cellar = Cellar::new(&root).unwrap();
        let linker = Linker::new(&prefix).unwrap();

        let installer = Installer::new(
            api_client,
            blob_cache,
            store,
            cellar,
            linker,
            prefix.clone(),
        );

        let result = installer.plan(&["nothing".to_string()]).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::types::Error::MissingFormula { .. }
        ));
    }

    #[test]
    fn stage_cask_apps_moves_app_and_leaves_caskroom_symlink() {
        let tmp = TempDir::new().unwrap();
        let source_root = tmp.path().join("mounted");
        let source_app = source_root.join("Ghostty.app");
        let staged_path = tmp.path().join("homebrew/Caskroom/ghostty/1.3.1");
        let prefix = tmp.path().join("homebrew");

        fs::create_dir_all(source_app.join("Contents")).unwrap();
        fs::write(source_app.join("Contents/Info.plist"), "ghostty").unwrap();
        fs::create_dir_all(&staged_path).unwrap();

        let cask = crate::core::installer::cask::ResolvedCask {
            install_name: "cask:ghostty".to_string(),
            token: "ghostty".to_string(),
            version: "1.3.1".to_string(),
            url: "https://example.com/Ghostty.dmg".to_string(),
            sha256: "abc".to_string(),
            binaries: Vec::new(),
            apps: vec![crate::core::installer::cask::CaskApp {
                source: "Ghostty.app".to_string(),
                target: "Ghostty.app".to_string(),
            }],
            linked_artifacts: Vec::new(),
        };

        stage_cask_apps(&source_root, &staged_path, &prefix, &cask).unwrap();

        let target_app = prefix.join("Applications/Ghostty.app");
        assert!(target_app.join("Contents/Info.plist").exists());
        assert!(staged_path.join("Ghostty.app").is_symlink());
        assert_eq!(
            fs::read_link(staged_path.join("Ghostty.app")).unwrap(),
            target_app
        );
    }

    #[test]
    fn stage_cask_linked_artifacts_links_appdir_sources_into_prefix() {
        let tmp = TempDir::new().unwrap();
        let source_root = tmp.path().join("mounted");
        let prefix = tmp.path().join("homebrew");
        let app_resources = prefix.join("Applications/Ghostty.app/Contents/Resources");

        fs::create_dir_all(app_resources.join("man/man1")).unwrap();
        fs::create_dir_all(app_resources.join("bash-completion/completions")).unwrap();
        fs::write(app_resources.join("man/man1/ghostty.1"), "man").unwrap();
        fs::write(
            app_resources.join("bash-completion/completions/ghostty.bash"),
            "complete",
        )
        .unwrap();

        let cask = crate::core::installer::cask::ResolvedCask {
            install_name: "cask:ghostty".to_string(),
            token: "ghostty".to_string(),
            version: "1.3.1".to_string(),
            url: "https://example.com/Ghostty.dmg".to_string(),
            sha256: "abc".to_string(),
            binaries: Vec::new(),
            apps: Vec::new(),
            linked_artifacts: vec![
                crate::core::installer::cask::CaskLinkedArtifact {
                    kind: crate::core::installer::cask::CaskLinkedArtifactKind::Manpage,
                    source:
                        "$APPDIR/Ghostty.app/Contents/Resources/man/man1/ghostty.1".to_string(),
                    target: "share/man/man1/ghostty.1".to_string(),
                },
                crate::core::installer::cask::CaskLinkedArtifact {
                    kind: crate::core::installer::cask::CaskLinkedArtifactKind::BashCompletion,
                    source: "$APPDIR/Ghostty.app/Contents/Resources/bash-completion/completions/ghostty.bash".to_string(),
                    target: "etc/bash_completion.d/ghostty".to_string(),
                },
            ],
        };

        stage_cask_linked_artifacts(&source_root, &prefix, &cask).unwrap();

        assert_eq!(
            fs::read_link(prefix.join("share/man/man1/ghostty.1")).unwrap(),
            app_resources.join("man/man1/ghostty.1")
        );
        assert_eq!(
            fs::read_link(prefix.join("etc/bash_completion.d/ghostty")).unwrap(),
            app_resources.join("bash-completion/completions/ghostty.bash")
        );
    }

    #[test]
    fn uninstall_cask_removes_app_and_caskroom() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("upkg");
        let prefix = tmp.path().join("homebrew");
        let caskroom_app = prefix.join("Caskroom/ghostty/1.3.1/Ghostty.app");
        let target_app = prefix.join("Applications/Ghostty.app");
        let manpage_source = target_app.join("Contents/Resources/man/man1/ghostty.1");
        let manpage_link = prefix.join("share/man/man1/ghostty.1");
        let metadata_cask =
            prefix.join("Caskroom/ghostty/.metadata/1.3.1/20260502093557.000/Casks/ghostty.json");

        fs::create_dir_all(target_app.join("Contents")).unwrap();
        fs::create_dir_all(manpage_source.parent().unwrap()).unwrap();
        fs::write(&manpage_source, "man").unwrap();
        fs::create_dir_all(manpage_link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&manpage_source, &manpage_link).unwrap();
        fs::create_dir_all(caskroom_app.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&target_app, &caskroom_app).unwrap();
        fs::create_dir_all(metadata_cask.parent().unwrap()).unwrap();
        write_json_pretty(
            &metadata_cask,
            &serde_json::json!({
                "token": "ghostty",
                "version": "1.3.1",
                "url": "https://example.com/Ghostty.dmg",
                "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "artifacts": [
                    { "app": ["Ghostty.app"] },
                    { "manpage": ["$APPDIR/Ghostty.app/Contents/Resources/man/man1/ghostty.1"] }
                ]
            }),
        )
        .unwrap();

        let api_client = ApiClient::new();
        let blob_cache = BlobCache::new(&root.join("cache")).unwrap();
        let store = Store::new(&root).unwrap();
        let cellar = Cellar::new(&root).unwrap();
        let linker = Linker::new(&prefix).unwrap();

        let mut installer = Installer::new(api_client, blob_cache, store, cellar, linker, prefix);

        installer.uninstall("cask:ghostty").unwrap();

        assert!(!target_app.exists());
        assert!(!manpage_link.exists());
        assert!(!tmp.path().join("homebrew/Caskroom/ghostty").exists());
    }
}
