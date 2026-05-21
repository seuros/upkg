use crate::error::UpkgError;

fn install_options(kind: crate::cli::PackageKind) -> crate::api::InstallOptions {
    crate::api::InstallOptions {
        package_kind: match kind {
            crate::cli::PackageKind::Auto => crate::api::PackageKindHint::Auto,
            crate::cli::PackageKind::App => crate::api::PackageKindHint::App,
        },
        ..crate::api::InstallOptions::default()
    }
}

pub fn install_native(packages: &[String], kind: crate::cli::PackageKind) -> Result<(), UpkgError> {
    let options = install_options(kind);
    crate::api::install(packages, &options).map_err(UpkgError::Native)
}

pub fn uninstall_native(
    packages: &[String],
    kind: crate::cli::PackageKind,
) -> Result<(), UpkgError> {
    let options = install_options(kind);
    crate::api::uninstall(packages, &options).map_err(UpkgError::Native)
}

pub fn upgrade_native(packages: &[String], kind: crate::cli::PackageKind) -> Result<(), UpkgError> {
    let options = install_options(kind);
    crate::api::upgrade(packages, &options).map_err(UpkgError::Native)
}

pub fn list_native() -> Result<(), UpkgError> {
    let options = crate::api::InstallOptions::default();
    let installed = crate::api::list(&options).map_err(UpkgError::Native)?;

    if installed.is_empty() {
        return Ok(());
    }

    for package in installed {
        let kind = if package.name.starts_with("cask:") {
            "app"
        } else {
            "formula"
        };
        println!("{kind}\t{}\t{}", package.name, package.version);
    }

    Ok(())
}
