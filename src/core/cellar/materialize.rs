use crate::types::Error;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
use crate::core::extraction::patch::linux::patch_placeholders;

#[cfg(target_os = "macos")]
use crate::core::extraction::patch::macos::{
    codesign_and_strip_xattrs, patch_homebrew_placeholders,
};

pub struct Cellar {
    cellar_dir: PathBuf,
}

impl Cellar {
    #[cfg(test)]
    pub fn new(root: &Path) -> io::Result<Self> {
        Self::new_at(root.join("Cellar"))
    }

    pub fn new_at(cellar_dir: PathBuf) -> io::Result<Self> {
        fs::create_dir_all(&cellar_dir)?;
        Ok(Self { cellar_dir })
    }

    pub fn keg_path(&self, name: &str, version: &str) -> PathBuf {
        self.cellar_dir.join(name).join(version)
    }

    pub fn root_dir(&self) -> &Path {
        &self.cellar_dir
    }

    #[cfg(test)]
    pub fn has_keg(&self, name: &str, version: &str) -> bool {
        self.keg_path(name, version).exists()
    }

    pub fn materialize(
        &self,
        name: &str,
        version: &str,
        store_entry: &Path,
    ) -> Result<PathBuf, Error> {
        let keg_path = self.keg_path(name, version);

        if keg_path.exists() {
            return Ok(keg_path);
        }

        if let Some(parent) = keg_path.parent() {
            fs::create_dir_all(parent).map_err(|e| Error::StoreCorruption {
                message: format!("failed to create keg parent directory: {e}"),
            })?;
        }

        let src_path = find_bottle_content(store_entry, name, version)?;

        copy_dir_with_fallback(&src_path, &keg_path)?;

        #[cfg(target_os = "macos")]
        patch_homebrew_placeholders(&keg_path, &self.cellar_dir, name, version)?;

        #[cfg(target_os = "linux")]
        {
            let prefix = self
                .cellar_dir
                .parent()
                .ok_or_else(|| Error::StoreCorruption {
                    message: format!(
                        "Invalid cellar directory (no parent): {}",
                        self.cellar_dir.display()
                    ),
                })?;
            patch_placeholders(&keg_path, prefix, name, version)?;
        }

        #[cfg(target_os = "macos")]
        codesign_and_strip_xattrs(&keg_path)?;

        Ok(keg_path)
    }

    pub fn remove_keg(&self, name: &str, version: &str) -> Result<(), Error> {
        let keg_path = self.keg_path(name, version);

        if !keg_path.exists() {
            return Ok(());
        }

        fs::remove_dir_all(&keg_path).map_err(|e| Error::StoreCorruption {
            message: format!("failed to remove keg: {e}"),
        })?;

        if let Some(parent) = keg_path.parent() {
            let _ = fs::remove_dir(parent); // Ignore error if not empty
        }

        Ok(())
    }
}

fn find_bottle_content(store_entry: &Path, name: &str, version: &str) -> Result<PathBuf, Error> {
    let expected_path = store_entry.join(name).join(version);
    if expected_path.exists() && expected_path.is_dir() {
        return Ok(expected_path);
    }

    let name_path = store_entry.join(name);
    if name_path.exists() && name_path.is_dir() {
        if let Ok(entries) = fs::read_dir(&name_path) {
            let dirs: Vec<_> = entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_dir())
                .collect();
            if dirs.len() == 1 {
                return Ok(dirs[0].path());
            }
        }
        return Ok(name_path);
    }

    Ok(store_entry.to_path_buf())
}

fn copy_dir_with_fallback(src: &Path, dst: &Path) -> Result<(), Error> {
    #[cfg(target_os = "macos")]
    {
        if try_clonefile_dir(src, dst).is_ok() {
            return Ok(());
        }
    }

    copy_dir_recursive(src, dst, true)
}

#[cfg(target_os = "macos")]
fn try_clonefile_dir(src: &Path, dst: &Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let src_cstr = CString::new(src.as_os_str().as_bytes())?;
    let dst_cstr = CString::new(dst.as_os_str().as_bytes())?;

    const CLONE_NOFOLLOW: u32 = 0x0001;

    unsafe extern "C" {
        fn clonefile(src: *const libc::c_char, dst: *const libc::c_char, flags: u32)
        -> libc::c_int;
    }

    let result = unsafe { clonefile(src_cstr.as_ptr(), dst_cstr.as_ptr(), CLONE_NOFOLLOW) };

    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn copy_dir_recursive(src: &Path, dst: &Path, try_hardlink: bool) -> Result<(), Error> {
    fs::create_dir_all(dst).map_err(|e| Error::StoreCorruption {
        message: format!("failed to create directory {}: {e}", dst.display()),
    })?;

    for entry in fs::read_dir(src).map_err(|e| Error::StoreCorruption {
        message: format!("failed to read directory {}: {e}", src.display()),
    })? {
        let entry = entry.map_err(|e| Error::StoreCorruption {
            message: format!("failed to read directory entry: {e}"),
        })?;

        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        let file_type = entry.file_type().map_err(|e| Error::StoreCorruption {
            message: format!("failed to get file type: {e}"),
        })?;

        if file_type.is_dir() {
            copy_dir_recursive(&src_path, &dst_path, try_hardlink)?;
        } else if file_type.is_symlink() {
            let target = fs::read_link(&src_path).map_err(|e| Error::StoreCorruption {
                message: format!("failed to read symlink: {e}"),
            })?;

            #[cfg(unix)]
            std::os::unix::fs::symlink(&target, &dst_path).map_err(|e| Error::StoreCorruption {
                message: format!("failed to create symlink: {e}"),
            })?;

            #[cfg(not(unix))]
            fs::copy(&src_path, &dst_path).map_err(|e| Error::StoreCorruption {
                message: format!("failed to copy symlink as file: {e}"),
            })?;
        } else {
            if try_hardlink && fs::hard_link(&src_path, &dst_path).is_ok() {
                continue;
            }

            fs::copy(&src_path, &dst_path).map_err(|e| Error::StoreCorruption {
                message: format!("failed to copy file: {e}"),
            })?;

            #[cfg(unix)]
            {
                let metadata = fs::metadata(&src_path).map_err(|e| Error::StoreCorruption {
                    message: format!("failed to read metadata: {e}"),
                })?;
                fs::set_permissions(&dst_path, metadata.permissions()).map_err(|e| {
                    Error::StoreCorruption {
                        message: format!("failed to set permissions: {e}"),
                    }
                })?;
            }
        }
    }

    Ok(())
}

#[cfg(all(test, target_os = "macos"))]
fn copy_dir_copy_only(src: &Path, dst: &Path) -> Result<(), Error> {
    copy_dir_recursive(src, dst, false)
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;

    fn setup_store_entry(tmp: &TempDir) -> PathBuf {
        let store_entry = tmp.path().join("store/abc123");

        fs::create_dir_all(store_entry.join("bin")).unwrap();
        fs::create_dir_all(store_entry.join("lib")).unwrap();

        fs::write(store_entry.join("bin/foo"), b"#!/bin/sh\necho foo").unwrap();
        let mut perms = fs::metadata(store_entry.join("bin/foo"))
            .unwrap()
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(store_entry.join("bin/foo"), perms).unwrap();

        fs::write(store_entry.join("lib/libfoo.dylib"), b"fake dylib").unwrap();

        std::os::unix::fs::symlink("libfoo.dylib", store_entry.join("lib/libfoo.1.dylib")).unwrap();

        store_entry
    }

    #[test]
    fn tree_reproduced_exactly() {
        let tmp = TempDir::new().unwrap();
        let store_entry = setup_store_entry(&tmp);

        let cellar = Cellar::new(tmp.path()).unwrap();
        let keg_path = cellar.materialize("foo", "1.2.3", &store_entry).unwrap();

        assert!(keg_path.exists());
        assert!(keg_path.join("bin").exists());
        assert!(keg_path.join("lib").exists());

        assert_eq!(
            fs::read_to_string(keg_path.join("bin/foo")).unwrap(),
            "#!/bin/sh\necho foo"
        );
        assert_eq!(
            fs::read(keg_path.join("lib/libfoo.dylib")).unwrap(),
            b"fake dylib"
        );

        let perms = fs::metadata(keg_path.join("bin/foo"))
            .unwrap()
            .permissions();
        assert!(perms.mode() & 0o111 != 0, "executable bit not preserved");

        let link_path = keg_path.join("lib/libfoo.1.dylib");
        assert!(
            link_path
                .symlink_metadata()
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read_link(&link_path).unwrap(),
            PathBuf::from("libfoo.dylib")
        );
    }

    #[test]
    fn second_materialize_is_noop() {
        let tmp = TempDir::new().unwrap();
        let store_entry = setup_store_entry(&tmp);

        let cellar = Cellar::new(tmp.path()).unwrap();

        let keg_path1 = cellar.materialize("foo", "1.2.3", &store_entry).unwrap();

        fs::write(keg_path1.join("marker.txt"), b"original").unwrap();

        let keg_path2 = cellar.materialize("foo", "1.2.3", &store_entry).unwrap();
        assert_eq!(keg_path1, keg_path2);

        assert!(keg_path2.join("marker.txt").exists());
    }

    #[test]
    fn remove_keg_cleans_up() {
        let tmp = TempDir::new().unwrap();
        let store_entry = setup_store_entry(&tmp);

        let cellar = Cellar::new(tmp.path()).unwrap();
        cellar.materialize("foo", "1.2.3", &store_entry).unwrap();

        assert!(cellar.has_keg("foo", "1.2.3"));

        cellar.remove_keg("foo", "1.2.3").unwrap();

        assert!(!cellar.has_keg("foo", "1.2.3"));
    }

    #[test]
    fn keg_path_format() {
        let tmp = TempDir::new().unwrap();
        let cellar = Cellar::new(tmp.path()).unwrap();

        let path = cellar.keg_path("libheif", "2.0.1");
        assert!(path.ends_with("Cellar/libheif/2.0.1"));
    }

    #[test]
    fn hardlink_fallback_to_copy_works() {
        let tmp1 = TempDir::new().unwrap();
        let tmp2 = TempDir::new().unwrap();

        let src = tmp1.path().join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("test.txt"), b"test content").unwrap();

        let dst = tmp2.path().join("dst");

        copy_dir_copy_only(&src, &dst).unwrap();

        assert_eq!(
            fs::read_to_string(dst.join("test.txt")).unwrap(),
            "test content"
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn clonefile_fallback_works() {
        let tmp = TempDir::new().unwrap();
        let store_entry = setup_store_entry(&tmp);

        let cellar = Cellar::new(tmp.path()).unwrap();
        let keg_path = cellar.materialize("clone", "1.0.0", &store_entry).unwrap();

        assert_eq!(
            fs::read_to_string(keg_path.join("bin/foo")).unwrap(),
            "#!/bin/sh\necho foo"
        );
    }

    #[test]
    fn version_mismatch_regex_fixes_paths() {
        use regex::Regex;

        let pkg_name = "ffmpeg";
        let pkg_version = "8.0.1_2";

        let version_pattern = format!(r"(/{}/)([^/]+)(/)", regex::escape(pkg_name));
        let version_regex = Regex::new(&version_pattern).unwrap();

        let old_path = "/opt/upkg/prefix/Cellar/ffmpeg/8.0.1_1/lib/libavdevice.62.dylib";
        let replacement = format!("/{}/{}/", pkg_name, pkg_version);

        let fixed = version_regex.replace(old_path, |caps: &regex::Captures| {
            let matched_version = &caps[2];
            if matched_version != pkg_version {
                replacement.clone()
            } else {
                caps[0].to_string()
            }
        });

        assert_eq!(
            fixed,
            "/opt/upkg/prefix/Cellar/ffmpeg/8.0.1_2/lib/libavdevice.62.dylib"
        );

        let correct_path = "/opt/upkg/prefix/Cellar/ffmpeg/8.0.1_2/lib/libavdevice.62.dylib";
        let fixed2 = version_regex.replace(correct_path, |caps: &regex::Captures| {
            let matched_version = &caps[2];
            if matched_version != pkg_version {
                replacement.clone()
            } else {
                caps[0].to_string()
            }
        });

        assert_eq!(fixed2, correct_path);

        let other_path = "/opt/upkg/prefix/Cellar/libvpx/1.0.0/lib/libvpx.dylib";
        let fixed3 = version_regex.replace(other_path, |caps: &regex::Captures| {
            let matched_version = &caps[2];
            if matched_version != pkg_version {
                replacement.clone()
            } else {
                caps[0].to_string()
            }
        });

        assert_eq!(fixed3, other_path);
    }
}
