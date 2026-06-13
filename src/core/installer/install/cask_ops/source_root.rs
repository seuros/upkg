use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
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
        || has_dmg_udif_trailer(path).unwrap_or(false)
}

fn has_dmg_udif_trailer(path: &Path) -> std::io::Result<bool> {
    const UDIF_TRAILER_SIZE: u64 = 512;
    const UDIF_TRAILER_MAGIC: &[u8; 4] = b"koly";

    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    if len < UDIF_TRAILER_SIZE {
        return Ok(false);
    }

    file.seek(SeekFrom::End(-(UDIF_TRAILER_SIZE as i64)))?;

    let mut magic = [0; 4];
    file.read_exact(&mut magic)?;

    Ok(&magic == UDIF_TRAILER_MAGIC)
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn detects_dmg_by_udif_trailer_when_url_and_cache_path_hide_extension() {
        let tmp = TempDir::new().unwrap();
        let blob_path = tmp.path().join("sha256.tar.gz");
        let mut bytes = vec![0; 1024];
        let trailer_start = bytes.len() - 512;
        bytes[trailer_start..trailer_start + 4].copy_from_slice(b"koly");
        fs::write(&blob_path, bytes).unwrap();

        assert!(is_dmg(
            "https://portswigger-cdn.net/burp/releases/download?product=community&type=MacOsArm64",
            &blob_path
        ));
    }

    #[test]
    fn does_not_detect_regular_tarball_cache_path_as_dmg() {
        let tmp = TempDir::new().unwrap();
        let blob_path = tmp.path().join("sha256.tar.gz");
        fs::write(&blob_path, vec![0; 1024]).unwrap();

        assert!(!is_dmg("https://example.com/archive.tar.gz", &blob_path));
    }
}
