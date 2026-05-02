use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::core::cellar::link::Linker;
use crate::core::cellar::materialize::Cellar;
use crate::types::Error;

use super::Installer;

#[path = "cask_ops/metadata.rs"]
mod metadata;
#[path = "cask_ops/source_root.rs"]
mod source_root;

#[cfg(test)]
pub(super) use metadata::write_json_pretty;
pub(super) use metadata::{
    cask_versions, load_latest_cask_metadata_json, write_brew_cask_metadata,
};
pub(super) use source_root::with_cask_source_root;

pub(super) struct FailedInstallGuard<'a> {
    linker: &'a Linker,
    cellar: &'a Cellar,
    name: &'a str,
    version: &'a str,
    keg_path: &'a Path,
    unlink: bool,
    armed: bool,
}

impl<'a> FailedInstallGuard<'a> {
    pub(super) fn new(
        linker: &'a Linker,
        cellar: &'a Cellar,
        name: &'a str,
        version: &'a str,
        keg_path: &'a Path,
        unlink: bool,
    ) -> Self {
        Self {
            linker,
            cellar,
            name,
            version,
            keg_path,
            unlink,
            armed: true,
        }
    }

    pub(super) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for FailedInstallGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            Installer::cleanup_failed_install(
                self.linker,
                self.cellar,
                self.name,
                self.version,
                self.keg_path,
                self.unlink,
            );
        }
    }
}

pub(super) fn stage_cask_binaries(
    extracted_root: &Path,
    keg_path: &Path,
    cask: &crate::core::installer::cask::ResolvedCask,
) -> Result<(), Error> {
    let bin_dir = keg_path.join("bin");
    fs::create_dir_all(&bin_dir).map_err(|e| Error::StoreCorruption {
        message: format!("failed to create cask bin dir: {e}"),
    })?;

    for binary in &cask.binaries {
        let source = resolve_cask_source_path(extracted_root, cask, &binary.source)?;
        if !source.exists() {
            return Err(Error::InvalidArgument {
                message: format!(
                    "cask '{}' binary source '{}' not found in archive",
                    cask.token, binary.source
                ),
            });
        }

        let target = bin_dir.join(&binary.target);
        if target.exists() {
            fs::remove_file(&target).map_err(|e| Error::StoreCorruption {
                message: format!("failed to replace existing cask binary: {e}"),
            })?;
        }

        fs::copy(&source, &target).map_err(|e| Error::StoreCorruption {
            message: format!("failed to stage cask binary '{}': {e}", binary.target),
        })?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&target)
                .map_err(|e| Error::StoreCorruption {
                    message: format!("failed to read staged cask binary metadata: {e}"),
                })?
                .permissions();
            if perms.mode() & 0o111 == 0 {
                perms.set_mode(0o755);
                fs::set_permissions(&target, perms).map_err(|e| Error::StoreCorruption {
                    message: format!("failed to make staged cask binary executable: {e}"),
                })?;
            }
        }
    }

    Ok(())
}

fn resolve_cask_source_path(
    extracted_root: &Path,
    cask: &crate::core::installer::cask::ResolvedCask,
    source: &str,
) -> Result<std::path::PathBuf, Error> {
    if source.starts_with("$APPDIR") {
        return Err(Error::InvalidArgument {
            message: format!(
                "cask '{}' uses APPDIR artifacts which are not supported yet",
                cask.token
            ),
        });
    }

    let mut normalized = source.to_string();
    let caskroom_prefix = format!("$HOMEBREW_PREFIX/Caskroom/{}/{}/", cask.token, cask.version);
    if let Some(stripped) = normalized.strip_prefix(&caskroom_prefix) {
        normalized = stripped.to_string();
    }

    let source_path = Path::new(&normalized);
    if source_path.is_absolute() {
        return Err(Error::InvalidArgument {
            message: format!(
                "cask '{}' binary source '{}' must be a relative path",
                cask.token, source
            ),
        });
    }

    for component in source_path.components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err(Error::InvalidArgument {
                message: format!(
                    "cask '{}' binary source '{}' cannot contain '..'",
                    cask.token, source
                ),
            });
        }
    }

    Ok(extracted_root.join(source_path))
}

fn resolve_cask_link_source_path(
    source_root: &Path,
    prefix: &Path,
    cask: &crate::core::installer::cask::ResolvedCask,
    source: &str,
) -> Result<PathBuf, Error> {
    if source == "$APPDIR" {
        return Ok(cask_app_dir(prefix));
    }
    if let Some(stripped) = source.strip_prefix("$APPDIR/") {
        return Ok(cask_app_dir(prefix).join(stripped));
    }

    resolve_cask_source_path(source_root, cask, source)
}

pub(super) fn stage_cask_apps(
    source_root: &Path,
    staged_path: &Path,
    prefix: &Path,
    cask: &crate::core::installer::cask::ResolvedCask,
) -> Result<(), Error> {
    let app_dir = cask_app_dir(prefix);
    fs::create_dir_all(&app_dir).map_err(|e| Error::StoreCorruption {
        message: format!(
            "failed to create app directory '{}': {e}",
            app_dir.display()
        ),
    })?;

    for app in &cask.apps {
        let source = resolve_cask_source_path(source_root, cask, &app.source)?;
        if !source.exists() {
            return Err(Error::InvalidArgument {
                message: format!(
                    "cask '{}' app source '{}' not found",
                    cask.token, app.source
                ),
            });
        }

        let staged_app = staged_path.join(&app.target);
        let target = app_dir.join(&app.target);

        if target.exists() {
            return Err(Error::InvalidArgument {
                message: format!(
                    "app '{}' already exists at '{}'",
                    app.target,
                    target.display()
                ),
            });
        }

        remove_path_if_exists(&staged_app)?;
        copy_path_preserving_metadata(&source, &staged_app)?;
        move_path_preserving_metadata(&staged_app, &target)?;

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&target, &staged_app).map_err(|e| {
                Error::StoreCorruption {
                    message: format!(
                        "failed to link staged app '{}' to '{}': {e}",
                        staged_app.display(),
                        target.display()
                    ),
                }
            })?;
        }
    }

    Ok(())
}

pub(super) fn stage_cask_linked_artifacts(
    source_root: &Path,
    prefix: &Path,
    cask: &crate::core::installer::cask::ResolvedCask,
) -> Result<(), Error> {
    for artifact in &cask.linked_artifacts {
        let source = resolve_cask_link_source_path(source_root, prefix, cask, &artifact.source)?;
        if !source.exists() {
            return Err(Error::InvalidArgument {
                message: format!(
                    "cask '{}' artifact source '{}' not found",
                    cask.token, artifact.source
                ),
            });
        }

        let target = cask_prefix_target(prefix, &artifact.target)?;
        if target.exists() && !target.is_symlink() {
            return Err(Error::InvalidArgument {
                message: format!(
                    "cask '{}' artifact target '{}' already exists",
                    cask.token,
                    target.display()
                ),
            });
        }

        if target.is_symlink() {
            remove_path_if_exists(&target)?;
        }

        let parent = target.parent().ok_or_else(|| Error::StoreCorruption {
            message: format!("target '{}' has no parent", target.display()),
        })?;
        fs::create_dir_all(parent).map_err(|e| Error::StoreCorruption {
            message: format!("failed to create cask artifact target directory: {e}"),
        })?;

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&source, &target).map_err(|e| Error::StoreCorruption {
                message: format!(
                    "failed to link cask artifact '{}' to '{}': {e}",
                    target.display(),
                    source.display()
                ),
            })?;
        }
    }

    Ok(())
}

pub(super) fn remove_cask_linked_artifacts(
    prefix: &Path,
    cask: &crate::core::installer::cask::ResolvedCask,
) -> Result<(), Error> {
    for artifact in &cask.linked_artifacts {
        let target = cask_prefix_target(prefix, &artifact.target)?;
        if target.is_symlink() {
            remove_path_if_exists(&target)?;
        }
    }

    Ok(())
}

fn cask_prefix_target(prefix: &Path, target: &str) -> Result<PathBuf, Error> {
    let target_path = Path::new(target);
    if target_path.is_absolute()
        || target_path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(Error::InvalidArgument {
            message: format!("unsupported cask artifact target '{target}'"),
        });
    }

    Ok(prefix.join(target_path))
}

pub(super) fn cask_app_dir(_prefix: &Path) -> PathBuf {
    if let Some(path) = std::env::var_os("UPKG_APPDIR") {
        return PathBuf::from(path);
    }

    #[cfg(test)]
    {
        _prefix.join("Applications")
    }

    #[cfg(not(test))]
    {
        PathBuf::from("/Applications")
    }
}

pub(super) fn remove_path_if_exists(path: &Path) -> Result<(), Error> {
    if !path.exists() && !path.is_symlink() {
        return Ok(());
    }

    let metadata = fs::symlink_metadata(path).map_err(|e| Error::StoreCorruption {
        message: format!("failed to read path '{}': {e}", path.display()),
    })?;

    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
    .map_err(|e| Error::StoreCorruption {
        message: format!("failed to remove '{}': {e}", path.display()),
    })
}

fn copy_path_preserving_metadata(source: &Path, target: &Path) -> Result<(), Error> {
    let parent = target.parent().ok_or_else(|| Error::StoreCorruption {
        message: format!("target '{}' has no parent", target.display()),
    })?;
    fs::create_dir_all(parent).map_err(|e| Error::StoreCorruption {
        message: format!("failed to create target parent '{}': {e}", parent.display()),
    })?;

    let output = Command::new("/bin/cp")
        .arg("-pR")
        .arg(source)
        .arg(target)
        .output()
        .map_err(|e| Error::ExecutionError {
            message: format!("failed to run cp: {e}"),
        })?;

    if output.status.success() {
        Ok(())
    } else {
        Err(Error::ExecutionError {
            message: format!(
                "failed to copy '{}' to '{}': {}",
                source.display(),
                target.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        })
    }
}

fn move_path_preserving_metadata(source: &Path, target: &Path) -> Result<(), Error> {
    if fs::rename(source, target).is_ok() {
        return Ok(());
    }

    copy_path_preserving_metadata(source, target)?;
    remove_path_if_exists(source)
}
