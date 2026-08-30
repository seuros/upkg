use std::sync::Arc;

use crate::core::network::download::{DownloadProgressCallback, DownloadRequest, DownloadResult};
use crate::core::progress::{InstallProgress, ProgressCallback};
use crate::types::{Error, Formula, InstallMethod, SelectedBottle};

use super::{ExecuteResult, InstallPlan, Installer, MAX_CORRUPTION_RETRIES};

impl Installer {
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
}
