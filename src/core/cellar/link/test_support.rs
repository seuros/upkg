//! Keg fixtures shared by linker and declarative post-install tests.

use std::fs;
use std::path::{Path, PathBuf};

/// Creates `<prefix>/Cellar/<keg>` holding `files` as `(relative path, contents)`.
pub(super) fn write_keg(prefix: &Path, keg: &str, files: &[(&str, &[u8])]) -> PathBuf {
    let keg = prefix.join("Cellar").join(keg);
    for (name, data) in files {
        let path = keg.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, data).unwrap();
    }
    keg
}
