use std::path::PathBuf;
use std::sync::Arc;

use crate::core::progress::ProgressCallback;
use crate::core::storage::receipt::{InstallReceipt, read_receipt};
use crate::types::{Error, formula_token};

use super::{ExecuteResult, InstallPlan, Installer};

#[derive(Debug)]
pub struct ReinstallPlan {
    /// Kegs rebuilt from the bottle already in the store: same version, same
    /// bytes, no network.
    pub from_store: Vec<StoreReinstall>,
    /// Requested names that have to be fetched again (no receipt, or the
    /// store entry is gone).
    pub fetch_names: Vec<String>,
    pub fetch: InstallPlan,
}

#[derive(Debug)]
pub struct StoreReinstall {
    pub keg_name: String,
    pub receipt: InstallReceipt,
}

/// An installed keg moved out of the way so its replacement can materialize
/// at the same path.
struct StashedKeg {
    install_name: String,
    keg_name: String,
    version: String,
    keg_path: PathBuf,
    backup_path: Option<PathBuf>,
    was_linked: bool,
}

impl Installer {
    /// Plans a reinstall of `names`. Every name must already be installed.
    ///
    /// A keg whose bottle is still in the store is rebuilt from it, which
    /// reinstalls exactly the installed version. Anything else is fetched,
    /// with missing dependencies planned and installed ones left alone.
    pub async fn plan_reinstall(
        &self,
        names: &[String],
        build_from_source: bool,
    ) -> Result<ReinstallPlan, Error> {
        let mut from_store = Vec::new();
        let mut fetch_names = Vec::new();

        for name in names {
            let installed = self.installed_keg(name)?;
            let keg_name = formula_token(&installed.name).to_string();
            let receipt = read_receipt(&self.cellar.keg_path(&keg_name, &installed.version));

            match receipt {
                Some(receipt)
                    if !build_from_source
                        && !receipt.store_key.is_empty()
                        && self.store.has_entry(&receipt.store_key) =>
                {
                    from_store.push(StoreReinstall { keg_name, receipt });
                }
                _ => fetch_names.push(name.clone()),
            }
        }

        let fetch = if fetch_names.is_empty() {
            InstallPlan { items: Vec::new() }
        } else {
            self.plan_forcing(&fetch_names, build_from_source, &fetch_names, false)
                .await?
        };

        Ok(ReinstallPlan {
            from_store,
            fetch_names,
            fetch,
        })
    }

    /// Replaces the installed kegs in `plan` with fresh copies.
    ///
    /// Each old keg is unlinked and set aside first. A keg whose replacement
    /// did not get a receipt is put back and relinked, so a failed reinstall
    /// leaves the previous install in place.
    pub async fn reinstall_with_progress(
        &mut self,
        plan: ReinstallPlan,
        link: bool,
        progress: Option<Arc<ProgressCallback>>,
    ) -> Result<ExecuteResult, Error> {
        let mut installed = 0usize;
        let mut first_error = None;

        for item in &plan.from_store {
            match self.reinstall_from_store(item, link) {
                Ok(()) => installed += 1,
                Err(e) => {
                    first_error.get_or_insert(e);
                }
            }
        }

        if !plan.fetch.items.is_empty() {
            match self
                .reinstall_fetched(plan.fetch, &plan.fetch_names, link, progress)
                .await
            {
                Ok(result) => installed += result.installed,
                Err(e) => {
                    first_error.get_or_insert(e);
                }
            }
        }

        match first_error {
            Some(e) => Err(e),
            None => Ok(ExecuteResult { installed }),
        }
    }

    fn reinstall_from_store(&self, item: &StoreReinstall, link: bool) -> Result<(), Error> {
        let receipt = &item.receipt;
        let stashed = self.stash_keg(&receipt.install_name)?;

        let materialized = self
            .cellar
            .materialize(
                &item.keg_name,
                &receipt.version,
                &self.store.entry_path(&receipt.store_key),
            )
            .and_then(|keg_path| {
                self.record_install_receipt(
                    &keg_path,
                    &receipt.install_name,
                    &receipt.formula_name,
                    &receipt.version,
                    &receipt.store_key,
                )
                .map(|()| keg_path)
            });

        match materialized {
            Ok(keg_path) => {
                self.relink(&receipt.install_name, &keg_path, link && stashed.was_linked);
                self.discard_backup(stashed);
                Ok(())
            }
            Err(e) => {
                // Restoring clears whatever the failed attempt left at the keg path.
                self.restore(stashed, link);
                Err(e)
            }
        }
    }

    async fn reinstall_fetched(
        &mut self,
        plan: InstallPlan,
        names: &[String],
        link: bool,
        progress: Option<Arc<ProgressCallback>>,
    ) -> Result<ExecuteResult, Error> {
        let replacements: Vec<(String, PathBuf)> = plan
            .items
            .iter()
            .filter(|item| names.contains(&item.install_name))
            .map(|item| {
                (
                    item.install_name.clone(),
                    self.cellar
                        .keg_path(&item.formula.name, &item.formula.effective_version()),
                )
            })
            .collect();

        let mut stashed = Vec::with_capacity(names.len());
        for name in names {
            match self.stash_keg(name) {
                Ok(keg) => stashed.push(keg),
                Err(e) => {
                    for keg in stashed {
                        self.restore(keg, link);
                    }
                    return Err(e);
                }
            }
        }

        let result = self.execute_with_progress(plan, link, progress).await;

        let mut restored = Vec::new();
        for keg in stashed {
            let new_path = replacements
                .iter()
                .find(|(name, _)| *name == keg.install_name)
                .map(|(_, path)| path);

            if new_path.is_some_and(|path| read_receipt(path).is_some()) {
                self.discard_backup(keg);
                continue;
            }

            if let Some(new_path) = new_path
                && *new_path != keg.keg_path
                && new_path.exists()
                && let Some(version) = new_path.file_name().and_then(|v| v.to_str())
            {
                Self::cleanup_materialized(&self.cellar, &keg.keg_name, version);
            }
            restored.push(keg.install_name.clone());
            self.restore(keg, link);
        }

        match result {
            Err(e) => Err(e),
            Ok(_) if !restored.is_empty() => Err(Error::ExecutionError {
                message: format!(
                    "reinstall did not replace {}; previous install restored",
                    restored.join(", ")
                ),
            }),
            Ok(r) => Ok(r),
        }
    }

    fn stash_keg(&self, name: &str) -> Result<StashedKeg, Error> {
        let installed = self.installed_keg(name)?;
        let keg_name = formula_token(&installed.name).to_string();
        let keg_path = self.cellar.keg_path(&keg_name, &installed.version);

        let was_linked = !self.linker.unlink_keg(&keg_path)?.is_empty();
        let backup_path =
            Self::backup_existing_source_keg(&keg_path, &keg_name, &installed.version)?;

        Ok(StashedKeg {
            install_name: name.to_string(),
            keg_name,
            version: installed.version,
            keg_path,
            backup_path,
            was_linked,
        })
    }

    fn discard_backup(&self, keg: StashedKeg) {
        if let Some(backup_path) = keg.backup_path
            && let Err(e) =
                Self::remove_source_keg_backup(&backup_path, &keg.keg_name, &keg.version)
        {
            eprintln!("warning: {e}");
        }
    }

    fn restore(&self, keg: StashedKeg, link: bool) {
        let Some(backup_path) = keg.backup_path else {
            return;
        };

        if let Err(e) = Self::restore_source_keg_from_backup(
            &keg.keg_path,
            &backup_path,
            &keg.keg_name,
            &keg.version,
        ) {
            eprintln!(
                "warning: {e}; previous keg left at {}",
                backup_path.display()
            );
            return;
        }

        self.relink(&keg.install_name, &keg.keg_path, link && keg.was_linked);
    }

    fn relink(&self, name: &str, keg_path: &std::path::Path, link: bool) {
        if let Err(e) = self.linker.link_opt(keg_path) {
            eprintln!("warning: failed to create opt link for {name}: {e}");
        }
        if link && let Err(e) = self.linker.link_keg(keg_path) {
            eprintln!("warning: failed to link {name}: {e}");
        }
    }
}
