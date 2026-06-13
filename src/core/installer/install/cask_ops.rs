use std::fs;
use std::io::Read;
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

pub(super) fn install_cask_pkgs(
    source_root: &Path,
    blob_path: &Path,
    cask: &crate::core::installer::cask::ResolvedCask,
) -> Result<(), Error> {
    for pkg in &cask.pkgs {
        let source = resolve_cask_source_path(source_root, cask, &pkg.source)?;
        let pkg_path = if source.exists() {
            source
        } else if is_pkg(blob_path) {
            blob_path.to_path_buf()
        } else {
            return Err(Error::InvalidArgument {
                message: format!(
                    "cask '{}' pkg source '{}' not found",
                    cask.token, pkg.source
                ),
            });
        };

        install_pkg(&pkg_path)?;
    }

    Ok(())
}

pub(super) fn uninstall_cask_pkgs(
    cask: &crate::core::installer::cask::ResolvedCask,
) -> Result<(), Error> {
    for pattern in &cask.uninstall.pkgutil {
        for pkg_id in pkgutil_ids(pattern)? {
            remove_pkgutil_files(&pkg_id)?;
            forget_pkgutil_id(&pkg_id)?;
        }
    }

    for target in &cask.uninstall.delete {
        remove_cask_delete_target(target)?;
    }

    Ok(())
}

fn install_pkg(path: &Path) -> Result<(), Error> {
    let installer_path = prepare_installer_pkg_path(path)?;
    let output = Command::new("/usr/sbin/installer")
        .args(["-pkg"])
        .arg(&installer_path.path)
        .args(["-target", "/"])
        .output()
        .map_err(|e| Error::ExecutionError {
            message: format!("failed to run installer: {e}"),
        })?;

    if output.status.success() {
        return Ok(());
    }

    let command = format!(
        "/usr/sbin/installer -pkg {} -target /",
        shell_quote_path(&installer_path.path)?
    );
    if crate::privilege_macos::escalate_privilege(&command).map_err(|e| Error::ExecutionError {
        message: format!("failed to request installer privileges: {e}"),
    })? {
        return Ok(());
    }

    Err(Error::ExecutionError {
        message: format!(
            "failed to install pkg '{}': {}",
            path.display(),
            command_output_message(&output)
        ),
    })
}

pub(in crate::core::installer::install) struct PreparedInstallerPkgPath {
    pub(in crate::core::installer::install) path: PathBuf,
    temp_dir: Option<PathBuf>,
}

impl Drop for PreparedInstallerPkgPath {
    fn drop(&mut self) {
        if let Some(temp_dir) = &self.temp_dir {
            let _ = fs::remove_dir_all(temp_dir);
        }
    }
}

pub(in crate::core::installer::install) fn prepare_installer_pkg_path(
    path: &Path,
) -> Result<PreparedInstallerPkgPath, Error> {
    if path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("pkg"))
        .unwrap_or(false)
    {
        return Ok(PreparedInstallerPkgPath {
            path: path.to_path_buf(),
            temp_dir: None,
        });
    }

    let temp_dir = std::env::temp_dir().join(format!(
        "upkg-pkg-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    fs::create_dir_all(&temp_dir).map_err(|e| Error::StoreCorruption {
        message: format!("failed to create pkg installer temp dir: {e}"),
    })?;

    let installer_path = temp_dir.join(
        path.file_stem()
            .and_then(|stem| stem.to_str())
            .filter(|stem| !stem.is_empty())
            .unwrap_or("package")
            .to_string()
            + ".pkg",
    );

    if std::os::unix::fs::symlink(path, &installer_path).is_err() {
        fs::copy(path, &installer_path).map_err(|e| Error::StoreCorruption {
            message: format!(
                "failed to prepare pkg '{}' for installer: {e}",
                path.display()
            ),
        })?;
    }

    Ok(PreparedInstallerPkgPath {
        path: installer_path,
        temp_dir: Some(temp_dir),
    })
}

fn command_output_message(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = stderr.trim();
    let stdout = stdout.trim();
    match (stderr.is_empty(), stdout.is_empty()) {
        (false, false) => format!("{stderr}\n{stdout}"),
        (false, true) => stderr.to_string(),
        (true, false) => stdout.to_string(),
        (true, true) => format!("exit status {}", output.status),
    }
}

pub(super) fn is_pkg(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("pkg"))
        .unwrap_or(false)
        || has_xar_magic(path).unwrap_or(false)
}

fn has_xar_magic(path: &Path) -> std::io::Result<bool> {
    let mut file = fs::File::open(path)?;
    let mut magic = [0; 4];
    file.read_exact(&mut magic)?;
    Ok(&magic == b"xar!")
}

fn pkgutil_ids(pattern: &str) -> Result<Vec<String>, Error> {
    let output = Command::new("/usr/sbin/pkgutil")
        .arg(format!("--pkgs={pattern}"))
        .output()
        .map_err(|e| Error::ExecutionError {
            message: format!("failed to run pkgutil --pkgs: {e}"),
        })?;

    if !output.status.success() {
        return Ok(Vec::new());
    }

    let ids: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect();

    Ok(ids)
}

fn remove_pkgutil_files(pkg_id: &str) -> Result<(), Error> {
    let Some(location) = pkgutil_location(pkg_id)? else {
        return Ok(());
    };

    let output = Command::new("/usr/sbin/pkgutil")
        .args(["--files", pkg_id])
        .output()
        .map_err(|e| Error::ExecutionError {
            message: format!("failed to run pkgutil --files for '{pkg_id}': {e}"),
        })?;

    if !output.status.success() {
        return Ok(());
    }

    for rel in String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let path = location.join(rel);
        if !path.exists() && !path.is_symlink() {
            continue;
        }
        let metadata = fs::symlink_metadata(&path).map_err(|e| Error::StoreCorruption {
            message: format!("failed to read pkgutil path '{}': {e}", path.display()),
        })?;
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            continue;
        }
        remove_path_or_escalate(&path)?;
    }

    Ok(())
}

fn pkgutil_location(pkg_id: &str) -> Result<Option<PathBuf>, Error> {
    let output = Command::new("/usr/sbin/pkgutil")
        .args(["--pkg-info", pkg_id])
        .output()
        .map_err(|e| Error::ExecutionError {
            message: format!("failed to run pkgutil --pkg-info for '{pkg_id}': {e}"),
        })?;

    if !output.status.success() {
        return Ok(None);
    }

    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Some(location) = line.strip_prefix("location: ") else {
            continue;
        };
        let location = location.trim();
        if location.is_empty() {
            return Ok(Some(PathBuf::from("/")));
        }
        let location = PathBuf::from(location);
        return Ok(Some(if location.is_absolute() {
            location
        } else {
            PathBuf::from("/").join(location)
        }));
    }

    Ok(Some(PathBuf::from("/")))
}

fn forget_pkgutil_id(pkg_id: &str) -> Result<(), Error> {
    let output = Command::new("/usr/sbin/pkgutil")
        .args(["--forget", pkg_id])
        .output()
        .map_err(|e| Error::ExecutionError {
            message: format!("failed to run pkgutil --forget for '{pkg_id}': {e}"),
        })?;

    if output.status.success() && !pkgutil_id_exists(pkg_id)? {
        Ok(())
    } else if crate::privilege_macos::escalate_privilege(&format!(
        "/usr/sbin/pkgutil --forget {}",
        shell_quote(pkg_id)?
    ))
    .map_err(|e| Error::ExecutionError {
        message: format!("failed to request pkgutil privileges for '{pkg_id}': {e}"),
    })? {
        if pkgutil_id_exists(pkg_id)? {
            Err(Error::ExecutionError {
                message: format!("pkgutil receipt '{pkg_id}' still exists after privileged forget"),
            })
        } else {
            Ok(())
        }
    } else {
        Err(Error::ExecutionError {
            message: format!(
                "failed to forget pkgutil receipt '{}': {}",
                pkg_id,
                command_output_message(&output)
            ),
        })
    }
}

fn pkgutil_id_exists(pkg_id: &str) -> Result<bool, Error> {
    let output = Command::new("/usr/sbin/pkgutil")
        .args(["--pkg-info", pkg_id])
        .output()
        .map_err(|e| Error::ExecutionError {
            message: format!("failed to run pkgutil --pkg-info for '{pkg_id}': {e}"),
        })?;
    Ok(output.status.success())
}

fn remove_cask_delete_target(target: &str) -> Result<(), Error> {
    if target.contains('*') || target.contains('?') || target.contains('[') {
        let command = format!(
            "/bin/zsh -f -c {} -- {}",
            shell_quote(
                "setopt NULL_GLOB; for pattern in \"$@\"; do for path in ${(~)pattern}; do rm -rf -- \"$path\"; done; done"
            )?,
            shell_quote(target)?
        );
        let output = Command::new("/bin/zsh")
            .args([
                "-f",
                "-c",
                "setopt NULL_GLOB; for pattern in \"$@\"; do for path in ${(~)pattern}; do rm -rf -- \"$path\"; done; done",
                "--",
            ])
            .arg(target)
            .output()
            .map_err(|e| Error::ExecutionError {
                message: format!("failed to remove cask delete target '{target}': {e}"),
            })?;
        return if output.status.success() {
            Ok(())
        } else if crate::privilege_macos::escalate_privilege(&command).map_err(|e| {
            Error::ExecutionError {
                message: format!("failed to request delete privileges for '{target}': {e}"),
            }
        })? {
            Ok(())
        } else {
            Err(Error::ExecutionError {
                message: format!(
                    "failed to remove cask delete target '{}': {}",
                    target,
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            })
        };
    }

    remove_path_or_escalate(Path::new(target))
}

fn remove_path_or_escalate(path: &Path) -> Result<(), Error> {
    match remove_path_if_exists(path) {
        Ok(()) => Ok(()),
        Err(err) => {
            let command = format!("rm -rf -- {}", shell_quote_path(path)?);
            if crate::privilege_macos::escalate_privilege(&command).map_err(|e| {
                Error::ExecutionError {
                    message: format!(
                        "failed to request delete privileges for '{}': {e}",
                        path.display()
                    ),
                }
            })? {
                Ok(())
            } else {
                Err(err)
            }
        }
    }
}

fn shell_quote_path(path: &Path) -> Result<String, Error> {
    let value = path.to_str().ok_or_else(|| Error::InvalidArgument {
        message: format!("path is not valid UTF-8: {}", path.display()),
    })?;
    shell_quote(value)
}

fn shell_quote(value: &str) -> Result<String, Error> {
    shlex::try_quote(value)
        .map(|quoted| quoted.into_owned())
        .map_err(|e| Error::InvalidArgument {
            message: format!("value cannot be shell quoted: {e}"),
        })
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
