use std::collections::BTreeMap;

use crate::core::storage::receipt::find_installed;
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

            match self.api_client.get_formula(normalized).await {
                Ok(_) => formulas.push((original.clone(), normalized.clone())),
                Err(Error::MissingFormula { .. }) => {
                    match self.api_client.get_cask(normalized).await {
                        Ok(_) => {
                            casks.push((original.clone(), format!("cask:{normalized}")));
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
}
