use crate::error::UpkgError;

pub fn install_native(packages: &[String]) -> Result<(), UpkgError> {
    let options = upkg_native::api::InstallOptions::default();
    upkg_native::api::install(packages, &options).map_err(UpkgError::Native)
}

pub fn uninstall_native(packages: &[String]) -> Result<(), UpkgError> {
    let options = upkg_native::api::InstallOptions::default();
    upkg_native::api::uninstall(packages, &options).map_err(UpkgError::Native)
}

pub fn upgrade_native(packages: &[String]) -> Result<(), UpkgError> {
    let options = upkg_native::api::InstallOptions::default();
    upkg_native::api::upgrade(packages, &options).map_err(UpkgError::Native)
}
