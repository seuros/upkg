use std::path::{Path, PathBuf};

use crate::Formula;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildSystem {
    Autoconf,
    Cmake,
    Meson,
    Make,
    RubyFormula,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallMethod {
    Bottle(crate::SelectedBottle),
    Source(BuildPlan),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildPlan {
    pub formula_name: String,
    pub version: String,
    pub source_url: String,
    pub source_checksum: Option<String>,
    pub ruby_source_path: Option<String>,
    pub build_dependencies: Vec<String>,
    pub runtime_dependencies: Vec<String>,
    pub detected_system: BuildSystem,
    pub prefix: PathBuf,
    pub cellar_path: PathBuf,
}

impl BuildPlan {
    pub fn from_formula(formula: &Formula, prefix: &Path) -> Option<Self> {
        let source = formula.source_url()?;
        let version = formula.effective_version();
        let cellar_path = prefix.join("Cellar").join(&formula.name).join(&version);

        let all_build_deps = formula.all_build_dependencies();
        let detected_system = detect_build_system(&source.url, &all_build_deps);

        Some(Self {
            formula_name: formula.name.clone(),
            version,
            source_url: source.url.clone(),
            source_checksum: source.checksum.clone(),
            ruby_source_path: formula.ruby_source_path.clone(),
            build_dependencies: all_build_deps,
            runtime_dependencies: formula.dependencies.clone(),
            detected_system,
            prefix: prefix.to_path_buf(),
            cellar_path,
        })
    }
}

fn detect_build_system(source_url: &str, build_deps: &[String]) -> BuildSystem {
    let has_dep = |name: &str| build_deps.iter().any(|d| d == name);

    if has_dep("cmake") {
        return BuildSystem::Cmake;
    }
    if has_dep("meson") {
        return BuildSystem::Meson;
    }
    if source_url.ends_with(".tar.gz")
        || source_url.ends_with(".tar.xz")
        || source_url.ends_with(".tar.bz2")
    {
        return BuildSystem::Autoconf;
    }
    BuildSystem::RubyFormula
}

#[cfg(test)]
#[path = "plan/tests.rs"]
mod tests;
