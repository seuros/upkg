use crate::api::PackageKindHint;
use crate::cli::{PackageAction, PackageKind};
use crate::error::UpkgError;

impl From<PackageKind> for PackageKindHint {
    fn from(kind: PackageKind) -> Self {
        match kind {
            PackageKind::Auto => Self::Auto,
            PackageKind::App => Self::App,
        }
    }
}

pub fn run(action: PackageAction, packages: &[String], kind: PackageKind) -> Result<(), UpkgError> {
    let options = crate::api::InstallOptions {
        package_kind: kind.into(),
        ..crate::api::InstallOptions::default()
    };
    match action {
        PackageAction::Install => crate::api::install(packages, &options),
        PackageAction::Uninstall => crate::api::uninstall(packages, &options),
        PackageAction::Upgrade => crate::api::upgrade(packages, &options),
        PackageAction::Reinstall => crate::api::reinstall(packages, &options),
    }
    .map_err(UpkgError::Native)
}

pub fn print_dry_run(
    action: PackageAction,
    packages: &[String],
    kind: PackageKind,
) -> Result<(), UpkgError> {
    let command = match action {
        PackageAction::Install => "install",
        PackageAction::Uninstall => "uninstall",
        PackageAction::Upgrade => "upgrade",
        PackageAction::Reinstall => "reinstall",
    };
    let package_args = match kind {
        PackageKind::Auto => packages.to_vec(),
        PackageKind::App => packages
            .iter()
            .map(|package| crate::package_ref::normalize_app_name(package))
            .collect::<Result<Vec<_>, _>>()
            .map_err(UpkgError::Native)?,
    };

    let mut rendered = vec!["upkg".to_string(), command.to_string()];
    if kind == PackageKind::App {
        rendered.push("--app".to_string());
    }
    rendered.extend(package_args);

    println!("engine: built-in macOS Homebrew-compatible");
    println!("dry-run: {}", rendered.join(" "));
    Ok(())
}

pub fn search_native(
    query: &str,
    exact: bool,
    kind: PackageKind,
    refresh: bool,
) -> Result<(), UpkgError> {
    let options = crate::api::SearchOptions {
        package_kind: kind.into(),
        exact,
        refresh,
        ..crate::api::SearchOptions::default()
    };

    let hits = crate::api::search(query, &options).map_err(UpkgError::Native)?;

    for hit in hits {
        let label = match hit.kind {
            crate::api::SearchKind::Formula => "formula",
            crate::api::SearchKind::Cask => "app",
        };
        let desc = hit.desc.as_deref().unwrap_or("");
        println!("{label}\t{}\t{}\t{}", hit.name, hit.version, desc);
    }

    Ok(())
}

pub fn list_native() -> Result<(), UpkgError> {
    let options = crate::api::InstallOptions::default();
    let installed = crate::api::list(&options).map_err(UpkgError::Native)?;

    if installed.is_empty() {
        return Ok(());
    }

    for package in installed {
        let kind = if crate::package_ref::is_cask_name(&package.name) {
            "app"
        } else {
            "formula"
        };
        println!("{kind}\t{}\t{}", package.name, package.version);
    }

    Ok(())
}
