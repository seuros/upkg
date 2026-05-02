use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::core::storage::store::Store;
use crate::types::Error;

pub(in crate::core::installer::install) fn with_cask_source_root(
    store: &Store,
    cask: &crate::core::installer::cask::ResolvedCask,
    blob_path: &Path,
    f: impl FnOnce(&Path) -> Result<(), Error>,
) -> Result<(), Error> {
    if is_dmg(&cask.url, blob_path) {
        let mounted = MountedDmg::attach(blob_path)?;
        f(&mounted.mountpoint)
    } else {
        let extracted = store.ensure_entry(&cask.sha256, blob_path)?;
        f(&extracted)
    }
}

fn is_dmg(url: &str, path: &Path) -> bool {
    url.to_ascii_lowercase().ends_with(".dmg")
        || path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("dmg"))
            .unwrap_or(false)
}

struct MountedDmg {
    mountpoint: PathBuf,
}

impl MountedDmg {
    fn attach(path: &Path) -> Result<Self, Error> {
        let mountpoint = std::env::temp_dir().join(format!(
            "upkg-dmg-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));

        fs::create_dir_all(&mountpoint).map_err(|e| Error::StoreCorruption {
            message: format!("failed to create dmg mountpoint: {e}"),
        })?;

        let output = Command::new("hdiutil")
            .args(["attach", "-nobrowse", "-readonly", "-mountpoint"])
            .arg(&mountpoint)
            .arg(path)
            .output()
            .map_err(|e| Error::ExecutionError {
                message: format!("failed to run hdiutil attach: {e}"),
            })?;

        if !output.status.success() {
            let _ = fs::remove_dir_all(&mountpoint);
            return Err(Error::ExecutionError {
                message: format!(
                    "failed to mount dmg '{}': {}",
                    path.display(),
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            });
        }

        Ok(Self { mountpoint })
    }
}

impl Drop for MountedDmg {
    fn drop(&mut self) {
        let _ = Command::new("hdiutil")
            .args(["detach", "-quiet"])
            .arg(&self.mountpoint)
            .status();
        let _ = fs::remove_dir_all(&self.mountpoint);
    }
}
