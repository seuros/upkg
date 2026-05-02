use std::fs;
use std::path::{Path, PathBuf};

use crate::core::cellar::materialize::Cellar;
use crate::core::progress::InstallProgress;
use crate::core::storage::receipt::find_installed;
use crate::types::{BuildPlan, Error};

use super::{Installer, PlannedInstall, dependency_cellar_path};

impl Installer {
    pub(super) async fn install_from_source(
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

    pub(super) fn backup_existing_source_keg(
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

    pub(super) fn restore_source_keg_from_backup(
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

    pub(super) fn cleanup_materialized(cellar: &Cellar, name: &str, version: &str) {
        if let Err(e) = cellar.remove_keg(name, version) {
            eprintln!(
                "warning: failed to remove keg for {}@{} after install error: {}",
                name, version, e
            );
        }
    }
}
