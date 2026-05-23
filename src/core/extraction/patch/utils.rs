use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

pub(crate) struct WritablePath {
    path: PathBuf,
    original_permissions: fs::Permissions,
    restore: bool,
}

impl WritablePath {
    pub(crate) fn new(path: &Path) -> io::Result<Self> {
        let metadata = fs::metadata(path)?;
        Self::from_metadata(path, &metadata)
    }

    pub(crate) fn from_metadata(path: &Path, metadata: &fs::Metadata) -> io::Result<Self> {
        let original_permissions = metadata.permissions();
        let original_mode = original_permissions.mode();
        let restore = original_mode & 0o200 == 0;

        if restore {
            let mut writable = original_permissions.clone();
            writable.set_mode(original_mode | 0o200);
            fs::set_permissions(path, writable)?;
        }

        Ok(Self {
            path: path.to_path_buf(),
            original_permissions,
            restore,
        })
    }

    pub(crate) fn original_permissions(&self) -> fs::Permissions {
        self.original_permissions.clone()
    }
}

impl Drop for WritablePath {
    fn drop(&mut self) {
        if self.restore {
            let _ = fs::set_permissions(&self.path, self.original_permissions.clone());
        }
    }
}

pub(crate) fn replace_with_temp<E>(
    path: &Path,
    write: impl FnOnce(&mut fs::File) -> Result<(), E>,
) -> Result<(), E>
where
    E: From<io::Error>,
{
    let temp_path = path.with_extension("tmp_patch");
    let result = (|| {
        let mut temp_file = fs::File::create(&temp_path).map_err(E::from)?;
        write(&mut temp_file)?;
        drop(temp_file);
        fs::rename(&temp_path, path).map_err(E::from)
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }

    result
}
