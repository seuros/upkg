use std::collections::BTreeMap;

use crate::core::storage::receipt::find_installed;
use crate::package_ref::cask_name;
use crate::types::{BuildPlan, Error, Formula, InstallMethod, resolve_closure, select_bottle};

use super::{AutoInstallTargets, InstallPlan, Installer, PlannedInstall};

impl Installer {
    #[cfg(test)]
    pub async fn plan(&self, names: &[String]) -> Result<InstallPlan, Error> {
        self.plan_with_options(names, false).await
    }

    pub async fn plan_with_options(
        &self,
        names: &[String],
        build_from_source: bool,
    ) -> Result<InstallPlan, Error> {
        self.plan_forcing(names, build_from_source, &[], true).await
    }

    /// Plans like `plan_with_options`, but `force` names are planned even
    /// when the same version is already installed. With `upgrade_installed`
    /// off, any other installed formula is left alone whatever its version.
    pub(super) async fn plan_forcing(
        &self,
        names: &[String],
        build_from_source: bool,
        force: &[String],
        upgrade_installed: bool,
    ) -> Result<InstallPlan, Error> {
        let formulas = self.fetch_all_formulas(names).await?;
        let ordered = resolve_closure(names, &formulas)?;

        let mut items = Vec::with_capacity(ordered.len());
        for install_name in ordered {
            let formula = formulas.get(&install_name).cloned().unwrap();
            if !force.contains(&install_name)
                && find_installed(self.cellar.root_dir(), &install_name)
                    .map(|installed| {
                        !upgrade_installed || installed.version == formula.effective_version()
                    })
                    .unwrap_or(false)
            {
                continue;
            }
            // Upgrading a dependency nobody asked for is only worth it when a
            // bottle exists. Building it from source instead would make an
            // unrelated install depend on that build, so keep what works.
            if !build_from_source
                && !names.contains(&install_name)
                && !force.contains(&install_name)
                && select_bottle(&formula).is_err()
                && let Some(installed) = find_installed(self.cellar.root_dir(), &install_name)
            {
                eprintln!(
                    "    Keeping {install_name} {} (no bottle for {} on this platform)",
                    installed.version,
                    formula.effective_version()
                );
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

    pub async fn resolve_auto_install_targets(
        &self,
        names: &[(String, String)],
    ) -> Result<AutoInstallTargets, Error> {
        let mut formulas = Vec::new();
        let mut casks = Vec::new();

        for (original, normalized) in names {
            if normalized.contains('/') {
                formulas.push((original.clone(), normalized.clone()));
                continue;
            }

            match self.fetch_formula_with_retry(normalized).await {
                Ok(_) => formulas.push((original.clone(), normalized.clone())),
                Err(Error::MissingFormula { .. }) => {
                    match self.api_client.get_cask(normalized).await {
                        Ok(cask) => {
                            let token = cask
                                .get("token")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or(normalized);
                            casks.push((original.clone(), cask_name(token)));
                        }
                        Err(Error::MissingFormula { .. }) => {
                            return Err(Error::MissingFormula {
                                name: normalized.clone(),
                            });
                        }
                        Err(e) => return Err(e),
                    }
                }
                Err(e) => return Err(e),
            }
        }

        Ok(AutoInstallTargets { formulas, casks })
    }

    async fn fetch_formula_with_retry(&self, name: &str) -> Result<Formula, Error> {
        use chrono_machines::{AsyncRetryable, ExponentialBackoff};

        const MAX_ATTEMPTS: u8 = 4;
        let backoff = ExponentialBackoff::new()
            .base_delay_ms(200)
            .multiplier(2.0)
            .max_delay_ms(5_000)
            .max_attempts(MAX_ATTEMPTS);

        let label = name.to_owned();
        (|| self.api_client.get_formula(name))
            .retry_async(backoff)
            .when(|e| matches!(e, Error::NetworkFailure { .. }))
            .notify(move |ctx| {
                eprintln!(
                    "    Network error fetching {label}, retrying in {}ms (attempt {}/{MAX_ATTEMPTS})",
                    ctx.next_delay_ms.unwrap_or_default(),
                    ctx.attempt,
                );
            })
            .call_async(|ms| tokio::time::sleep(std::time::Duration::from_millis(ms)))
            .await
            .map(|outcome| outcome.into_inner())
            .map_err(|err| {
                err.into_cause().unwrap_or_else(|| Error::NetworkFailure {
                    message: format!("failed to fetch {name} after retries"),
                })
            })
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
                .map(|n| self.fetch_formula_with_retry(n))
                .collect();

            let results = futures_util::future::join_all(futures).await;

            for (i, result) in results.into_iter().enumerate() {
                let formula = match result {
                    Ok(f) => f,
                    // An installed dependency still satisfies the install even
                    // when its current formula can't be read.
                    Err(e @ (Error::UnsupportedFormula { .. } | Error::MissingFormula { .. }))
                        if !names.contains(&batch[i])
                            && find_installed(self.cellar.root_dir(), &batch[i]).is_some() =>
                    {
                        eprintln!("    Keeping installed {} ({e})", batch[i]);
                        continue;
                    }
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
}
