use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::types::Error;

#[derive(Clone)]
pub struct BlobCache {
    blobs_dir: PathBuf,
    tmp_dir: PathBuf,
}

impl BlobCache {
    pub fn new(cache_root: &Path) -> io::Result<Self> {
        let blobs_dir = cache_root.join("blobs");
        let tmp_dir = cache_root.join("tmp");

        fs::create_dir_all(&blobs_dir)?;
        fs::create_dir_all(&tmp_dir)?;

        Ok(Self { blobs_dir, tmp_dir })
    }

    pub fn blob_path(&self, sha256: &str) -> PathBuf {
        self.blobs_dir.join(format!("{sha256}.tar.gz"))
    }

    pub fn has_blob(&self, sha256: &str) -> bool {
        self.blob_path(sha256).exists()
    }

    pub fn remove_blob(&self, sha256: &str) -> io::Result<bool> {
        let path = self.blob_path(sha256);
        if path.exists() {
            fs::remove_file(&path)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn start_write(&self, sha256: &str) -> io::Result<BlobWriter> {
        let final_path = self.blob_path(sha256);
        let unique_id = std::process::id();
        let thread_id = std::thread::current().id();
        let tmp_path = self
            .tmp_dir
            .join(format!("{sha256}.{unique_id}.{thread_id:?}.tar.gz.part"));

        let file = fs::File::create(&tmp_path)?;

        Ok(BlobWriter {
            file,
            tmp_path,
            final_path,
            committed: false,
        })
    }
}

pub struct BlobWriter {
    file: fs::File,
    tmp_path: PathBuf,
    final_path: PathBuf,
    committed: bool,
}

impl BlobWriter {
    pub fn commit(mut self) -> Result<PathBuf, Error> {
        self.file.flush().map_err(|e| Error::NetworkFailure {
            message: format!("failed to flush blob: {e}"),
        })?;

        if self.final_path.exists() {
            let _ = fs::remove_file(&self.tmp_path);
            self.committed = true;
            return Ok(self.final_path.clone());
        }

        match fs::rename(&self.tmp_path, &self.final_path) {
            Ok(()) => {}
            Err(_) if self.final_path.exists() => {
                let _ = fs::remove_file(&self.tmp_path);
            }
            Err(e) => {
                return Err(Error::NetworkFailure {
                    message: format!("failed to rename blob: {e}"),
                });
            }
        }

        self.committed = true;
        Ok(self.final_path.clone())
    }
}

impl Write for BlobWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.file.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl Drop for BlobWriter {
    fn drop(&mut self) {
        if !self.committed && self.tmp_path.exists() {
            let _ = fs::remove_file(&self.tmp_path);
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
#[path = "blob/tests.rs"]
mod tests;
