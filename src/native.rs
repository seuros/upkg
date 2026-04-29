use crate::error::UpkgError;

pub fn install_native(packages: &[String]) -> Result<(), UpkgError> {
    let options = crate::api::InstallOptions::default();
    crate::api::install(packages, &options).map_err(UpkgError::Native)
}

pub fn uninstall_native(packages: &[String]) -> Result<(), UpkgError> {
    let options = crate::api::InstallOptions::default();
    crate::api::uninstall(packages, &options).map_err(UpkgError::Native)
}

pub fn upgrade_native(packages: &[String]) -> Result<(), UpkgError> {
    let options = crate::api::InstallOptions::default();
    crate::api::upgrade(packages, &options).map_err(UpkgError::Native)
}
