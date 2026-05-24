use crate::core::extraction::patch::utils::{WritablePath, replace_with_temp};
use crate::types::Error;
use std::fs;
use std::path::{Path, PathBuf};

const HOMEBREW_PREFIXES: &[&str] = &[
    "/opt/homebrew",
    "/usr/local/Homebrew",
    "/usr/local",
    "/home/linuxbrew/.linuxbrew",
];

fn patch_text_file_strings(path: &Path, new_prefix: &str, new_cellar: &str) -> Result<(), Error> {
    let mut file = match fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Ok(()),
    };

    let mut buf = [0u8; 8192];
    let n = match std::io::Read::read(&mut file, &mut buf) {
        Ok(n) => n,
        Err(_) => return Ok(()),
    };

    if buf[..n].contains(&0) {
        return Ok(());
    }

    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Ok(()),
    };

    if !content.contains("@@HOMEBREW_")
        && !content.contains("/opt/homebrew")
        && !content.contains("/usr/local")
        && !content.contains("/home/linuxbrew")
    {
        return Ok(());
    }

    let mut new_content = content.clone();
    let mut changed = false;

    new_content = new_content
        .replace("@@HOMEBREW_PREFIX@@", new_prefix)
        .replace("@@HOMEBREW_CELLAR@@", new_cellar)
        .replace("@@HOMEBREW_REPOSITORY@@", new_prefix)
        .replace("@@HOMEBREW_LIBRARY@@", &format!("{}/Library", new_prefix))
        .replace("@@HOMEBREW_PERL@@", "/usr/bin/perl")
        .replace("@@HOMEBREW_JAVA@@", "/usr/bin/java");

    if new_content != content {
        changed = true;
    }

    for old_prefix in HOMEBREW_PREFIXES {
        if old_prefix == &new_prefix {
            continue;
        }
        let replaced = new_content.replace(old_prefix, new_prefix);
        if replaced != new_content {
            new_content = replaced;
            changed = true;
        }
    }

    if !changed {
        return Ok(());
    }

    let _writable = WritablePath::new(path).map_err(|e| Error::StoreCorruption {
        message: format!("failed to make writable: {e}"),
    })?;

    fs::write(path, new_content).map_err(|e| Error::StoreCorruption {
        message: format!("failed to write file: {e}"),
    })?;

    Ok(())
}

#[cfg(test)]
fn patch_macho_binary_strings(path: &Path, new_prefix: &str) -> Result<(), Error> {
    patch_macho_binary_strings_with_cellar(path, new_prefix, None)
}

fn patch_macho_binary_strings_with_cellar(
    path: &Path,
    new_prefix: &str,
    new_cellar: Option<&str>,
) -> Result<(), Error> {
    use std::io::{Read as _, Write as _};

    let metadata = fs::metadata(path).map_err(|e| Error::StoreCorruption {
        message: format!("failed to read metadata: {e}"),
    })?;
    let writable =
        WritablePath::from_metadata(path, &metadata).map_err(|e| Error::StoreCorruption {
            message: format!("failed to make writable: {e}"),
        })?;

    let mut file = fs::File::open(path).map_err(|e| Error::StoreCorruption {
        message: format!("failed to open file: {e}"),
    })?;
    let mut contents = Vec::new();
    file.read_to_end(&mut contents)
        .map_err(|e| Error::StoreCorruption {
            message: format!("failed to read file: {e}"),
        })?;
    drop(file);

    let original_contents = contents.clone();
    let mut patched = false;

    // Build replacement pairs: (old_bytes, new_bytes)
    let cellar_str = new_cellar
        .map(String::from)
        .unwrap_or_else(|| format!("{new_prefix}/Cellar"));
    let placeholder_pairs: Vec<(&[u8], &[u8])> = vec![
        (b"@@HOMEBREW_CELLAR@@" as &[u8], cellar_str.as_bytes()),
        (b"@@HOMEBREW_PREFIX@@" as &[u8], new_prefix.as_bytes()),
    ];

    for (old_bytes, new_bytes) in &placeholder_pairs {
        if new_bytes.len() > old_bytes.len() {
            continue;
        }
        let shrink = old_bytes.len() - new_bytes.len();
        let mut i = 0;
        while i + old_bytes.len() <= contents.len() {
            if contents[i..i + old_bytes.len()] == **old_bytes {
                // Find the end of the containing C string (next null byte)
                let str_end = contents[i + old_bytes.len()..]
                    .iter()
                    .position(|&b| b == 0)
                    .map(|p| i + old_bytes.len() + p)
                    .unwrap_or(contents.len());

                // Write new prefix
                contents[i..i + new_bytes.len()].copy_from_slice(new_bytes);
                // Shift suffix left to close the gap
                contents.copy_within(i + old_bytes.len()..str_end, i + new_bytes.len());
                // Null-pad the freed bytes at the end of the string field
                let pad_start = str_end - shrink;
                contents[pad_start..str_end].fill(0);

                patched = true;
                i += new_bytes.len() + (str_end - i - old_bytes.len());
            } else {
                i += 1;
            }
        }
    }

    for old_prefix in HOMEBREW_PREFIXES {
        if old_prefix == &new_prefix {
            continue;
        }

        let old_bytes = old_prefix.as_bytes();
        let new_bytes = new_prefix.as_bytes();

        if new_bytes.len() > old_bytes.len() {
            continue;
        }

        let shrink = old_bytes.len() - new_bytes.len();
        let mut i = 0;
        while i + old_bytes.len() <= contents.len() {
            if contents[i..i + old_bytes.len()] == *old_bytes
                && matches!(
                    contents.get(i + old_bytes.len()).copied(),
                    None | Some(0) | Some(b'/')
                )
            {
                // Find the end of the containing C string
                let str_end = contents[i + old_bytes.len()..]
                    .iter()
                    .position(|&b| b == 0)
                    .map(|p| i + old_bytes.len() + p)
                    .unwrap_or(contents.len());

                // Write new prefix
                contents[i..i + new_bytes.len()].copy_from_slice(new_bytes);
                // Shift suffix left to close the gap
                contents.copy_within(i + old_bytes.len()..str_end, i + new_bytes.len());
                // Null-pad the freed bytes at the end
                let pad_start = str_end - shrink;
                contents[pad_start..str_end].fill(0);

                patched = true;
                i += new_bytes.len();
            } else {
                i += 1;
            }
        }
    }

    if patched && contents != original_contents {
        replace_with_temp(path, |temp_file| temp_file.write_all(&contents)).map_err(|e| {
            Error::StoreCorruption {
                message: format!("failed to replace patched file: {e}"),
            }
        })?;

        fs::set_permissions(path, writable.original_permissions()).map_err(|e| {
            Error::StoreCorruption {
                message: format!("failed to restore permissions after patching: {e}"),
            }
        })?;

        match std::process::Command::new("codesign")
            .args(["--force", "--sign", "-", &path.to_string_lossy()])
            .output()
        {
            Ok(output) if !output.status.success() => {
                eprintln!(
                    "Warning: Failed to re-sign {}: {}",
                    path.display(),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            Err(e) => {
                eprintln!(
                    "Warning: Failed to execute codesign for {}: {}",
                    path.display(),
                    e
                );
            }
            _ => {}
        }
    }

    Ok(())
}

pub fn patch_homebrew_placeholders(
    keg_path: &Path,
    cellar_dir: &Path,
    pkg_name: &str,
    pkg_version: &str,
) -> Result<(), Error> {
    use rayon::prelude::*;
    use regex::Regex;
    use std::process::Command;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, Once};

    static CLT_CHECK: Once = Once::new();
    CLT_CHECK.call_once(|| {
        crate::privilege_macos::warn_if_xcode_clt_missing();
    });

    let prefix = cellar_dir.parent().unwrap_or(Path::new("/opt/homebrew"));

    let cellar_str = cellar_dir.to_string_lossy().to_string();
    let prefix_str = prefix.to_string_lossy().to_string();

    let version_pattern = format!(r"(/{}/)([^/]+)(/)", regex::escape(pkg_name));
    let version_regex = Regex::new(&version_pattern).ok();

    let macho_files: Vec<PathBuf> = walkdir::WalkDir::new(keg_path)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            if let Ok(data) = fs::read(e.path())
                && data.len() >= 4
            {
                let magic = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
                return matches!(
                    magic,
                    0xfeedface | 0xfeedfacf | 0xcafebabe | 0xcefaedfe | 0xcffaedfe
                );
            }
            false
        })
        .map(|e| e.path().to_path_buf())
        .collect();

    let patch_failures = AtomicUsize::new(0);
    let first_patch_error: Arc<Mutex<Option<Error>>> = Arc::new(Mutex::new(None));

    macho_files.par_iter().for_each(|path| {
        if let Err(e) = patch_macho_binary_strings_with_cellar(path, &prefix_str, Some(&cellar_str))
        {
            patch_failures.fetch_add(1, Ordering::Relaxed);
            if let Ok(mut guard) = first_patch_error.lock()
                && guard.is_none()
            {
                *guard = Some(e);
            }
        }
    });

    if let Ok(mut guard) = first_patch_error.lock()
        && let Some(e) = guard.take()
    {
        return Err(e);
    }

    let text_files: Vec<PathBuf> = walkdir::WalkDir::new(keg_path)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.path().to_path_buf())
        .collect();

    text_files.par_iter().for_each(|path| {
        let _ = patch_text_file_strings(path, &prefix_str, &cellar_str);
    });

    let patch_path = |old_path: &str| -> Option<String> {
        let mut new_path = old_path.to_string();
        let mut changed = false;

        if old_path.contains("@@HOMEBREW_CELLAR@@") || old_path.contains("@@HOMEBREW_PREFIX@@") {
            new_path = new_path
                .replace("@@HOMEBREW_CELLAR@@", &cellar_str)
                .replace("@@HOMEBREW_PREFIX@@", &prefix_str);
            changed = true;
        }

        if let Some(re) = &version_regex
            && re.is_match(&new_path)
        {
            let replacement = format!("/{}/{}/", pkg_name, pkg_version);
            let fixed = re.replace(&new_path, |caps: &regex::Captures| {
                let matched_version = &caps[2];
                if matched_version != pkg_version {
                    replacement.clone()
                } else {
                    caps[0].to_string()
                }
            });
            if fixed != new_path {
                new_path = fixed.to_string();
                changed = true;
            }
        }

        if changed && new_path != old_path {
            Some(new_path)
        } else {
            None
        }
    };

    macho_files.par_iter().for_each(|path| {
        let metadata = match fs::metadata(path) {
            Ok(m) => m,
            Err(_) => return,
        };
        let _writable = match WritablePath::from_metadata(path, &metadata) {
            Ok(writable) => writable,
            Err(_) => {
                patch_failures.fetch_add(1, Ordering::Relaxed);
                return;
            }
        };

        let mut patched_any = false;

        if let Ok(output) = Command::new("otool")
            .args(["-L", &path.to_string_lossy()])
            .output()
            && output.status.success()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                let line = line.trim();
                if let Some(old_path) = line.split_whitespace().next()
                    && let Some(new_path) = patch_path(old_path)
                {
                    let result = Command::new("install_name_tool")
                        .args(["-change", old_path, &new_path, &path.to_string_lossy()])
                        .output();
                    if result.is_ok() {
                        patched_any = true;
                    } else {
                        patch_failures.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }

        if let Ok(output) = Command::new("otool")
            .args(["-D", &path.to_string_lossy()])
            .output()
            && output.status.success()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines().skip(1) {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if let Some(new_id) = patch_path(line) {
                    let result = Command::new("install_name_tool")
                        .args(["-id", &new_id, &path.to_string_lossy()])
                        .output();
                    if result.is_ok() {
                        patched_any = true;
                    } else {
                        patch_failures.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }

        if patched_any {
            let _ = Command::new("codesign")
                .args(["--force", "--sign", "-", &path.to_string_lossy()])
                .output();
        }
    });

    let failures = patch_failures.load(Ordering::Relaxed);
    if failures > 0 {
        return Err(Error::StoreCorruption {
            message: format!(
                "failed to patch {} Mach-O files in {}",
                failures,
                keg_path.display()
            ),
        });
    }

    Ok(())
}

pub fn codesign_and_strip_xattrs(keg_path: &Path) -> Result<(), Error> {
    use rayon::prelude::*;
    use std::process::Command;

    let _ = Command::new("xattr")
        .args(["-rd", "com.apple.quarantine", &keg_path.to_string_lossy()])
        .stderr(std::process::Stdio::null())
        .output();
    let _ = Command::new("xattr")
        .args(["-rd", "com.apple.provenance", &keg_path.to_string_lossy()])
        .stderr(std::process::Stdio::null())
        .output();

    let bin_files: Vec<PathBuf> = walkdir::WalkDir::new(keg_path)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let path = e.path();
            path.is_file() && path.to_string_lossy().contains("/bin/")
        })
        .map(|e| e.path().to_path_buf())
        .collect();

    bin_files.par_iter().for_each(|path| {
        let data = match fs::read(path) {
            Ok(d) if d.len() >= 4 => d,
            _ => return,
        };
        let magic = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
        let is_macho = matches!(
            magic,
            0xfeedface | 0xfeedfacf | 0xcafebabe | 0xcefaedfe | 0xcffaedfe
        );
        if !is_macho {
            return;
        }

        let verify = Command::new("codesign")
            .args(["-v", &path.to_string_lossy()])
            .stderr(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .status();

        if verify.map(|s| s.success()).unwrap_or(false) {
            return; // Already signed
        }

        let metadata = match fs::metadata(path) {
            Ok(m) => m,
            Err(_) => return,
        };
        let _writable = WritablePath::from_metadata(path, &metadata).ok();

        let _ = Command::new("codesign")
            .args(["--force", "--sign", "-", &path.to_string_lossy()])
            .output();
    });

    Ok(())
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;

    #[test]
    fn test_patch_macho_preserves_execute_bit() {
        let tmp = TempDir::new().unwrap();
        let test_file = tmp.path().join("test_binary");

        let old_prefix = "/home/linuxbrew/.linuxbrew";
        let new_prefix = "/opt/upkg/prefix";

        let mut contents = Vec::new();
        contents.extend_from_slice(b"\xfe\xed\xfa\xcf");
        contents.extend_from_slice(old_prefix.as_bytes());
        contents.extend_from_slice(b"/bin/hello\0");

        fs::write(&test_file, &contents).unwrap();

        let mut perms = fs::metadata(&test_file).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&test_file, perms).unwrap();

        patch_macho_binary_strings(&test_file, new_prefix).unwrap();

        let mode = fs::metadata(&test_file).unwrap().permissions().mode();
        assert!(
            mode & 0o111 != 0,
            "execute bit lost after patching: mode = {:#o}",
            mode
        );
    }

    #[test]
    fn test_patch_macho_binary_strings() {
        let tmp = TempDir::new().unwrap();
        let test_file = tmp.path().join("test_binary");

        let old_prefix = "/home/linuxbrew/.linuxbrew";
        let new_prefix = "/opt/upkg/prefix";

        let mut contents = Vec::new();
        contents.extend_from_slice(b"\xfe\xed\xfa\xcf");
        contents.extend_from_slice(b"some random data\0");
        contents.extend_from_slice(old_prefix.as_bytes());
        contents.extend_from_slice(b"/opt/git/libexec/git-core\0");
        contents.extend_from_slice(b"more data\0");
        contents.extend_from_slice(old_prefix.as_bytes());
        contents.extend_from_slice(b"/lib/libfoo.dylib\0");
        contents.extend_from_slice(b"end\0");

        fs::write(&test_file, &contents).unwrap();

        let result = patch_macho_binary_strings(&test_file, new_prefix);
        assert!(result.is_ok());

        let patched = fs::read(&test_file).unwrap();
        let patched_str = String::from_utf8_lossy(&patched);

        assert!(patched_str.contains(new_prefix));
        assert!(!patched_str.contains(old_prefix));
    }

    #[test]
    fn test_patch_macho_skips_when_new_prefix_longer() {
        let tmp = TempDir::new().unwrap();
        let test_file = tmp.path().join("test_binary");

        let old_prefix = "/opt/homebrew";
        let new_prefix = "/opt/upkg/prefix";

        let mut contents = Vec::new();
        contents.extend_from_slice(b"\xfe\xed\xfa\xcf");
        contents.extend_from_slice(b"some random data\0");
        contents.extend_from_slice(old_prefix.as_bytes());
        contents.extend_from_slice(b"/opt/git/libexec/git-core\0");
        contents.extend_from_slice(b"more data\0");

        let original = contents.clone();
        fs::write(&test_file, &contents).unwrap();

        let result = patch_macho_binary_strings(&test_file, new_prefix);
        assert!(
            result.is_ok(),
            "should skip when new prefix is longer than old prefix"
        );

        let unchanged = fs::read(&test_file).unwrap();
        assert_eq!(
            unchanged, original,
            "binary must be unchanged when prefix cannot be expanded in-place"
        );
    }

    #[test]
    fn test_patch_text_file_strings() {
        let tmp = TempDir::new().unwrap();
        let test_file = tmp.path().join("test_script.sh");

        let content = r#"#!/bin/bash
export GIT_EXEC_PATH=/opt/homebrew/opt/git/libexec/git-core
export PREFIX=@@HOMEBREW_PREFIX@@
export CELLAR=@@HOMEBREW_CELLAR@@
export LIBRARY=@@HOMEBREW_LIBRARY@@
export PERL=@@HOMEBREW_PERL@@
echo "Hello from $PREFIX"
"#;

        fs::write(&test_file, content).unwrap();

        let new_prefix = "/opt/upkg/prefix";
        let new_cellar = format!("{}/Cellar", new_prefix);

        let result = patch_text_file_strings(&test_file, new_prefix, &new_cellar);
        assert!(result.is_ok());

        let patched = fs::read_to_string(&test_file).unwrap();
        assert!(patched.contains(new_prefix));
        assert!(!patched.contains("/opt/homebrew"));
        assert!(!patched.contains("@@HOMEBREW_"));
        assert!(patched.contains("/opt/upkg/prefix/opt/git/libexec/git-core"));
        assert!(patched.contains("/opt/upkg/prefix/Cellar"));
        assert!(patched.contains("/opt/upkg/prefix/Library"));
        assert!(patched.contains("/usr/bin/perl"));
    }
}
