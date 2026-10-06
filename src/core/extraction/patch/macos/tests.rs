use super::*;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use tempfile::TempDir;

#[test]
fn fix_version_segment_fixes_mismatched_paths() {
    let cases = [
        (
            "/opt/upkg/prefix/Cellar/ffmpeg/8.0.1_1/lib/libavdevice.62.dylib",
            Some("/opt/upkg/prefix/Cellar/ffmpeg/8.0.1_2/lib/libavdevice.62.dylib"),
        ),
        (
            "/opt/upkg/prefix/Cellar/ffmpeg/8.0.1_2/lib/libavdevice.62.dylib",
            None,
        ),
        (
            "/opt/upkg/prefix/Cellar/libvpx/1.0.0/lib/libvpx.dylib",
            None,
        ),
        ("/opt/upkg/prefix/Cellar/ffmpeg", None),
        ("/opt/upkg/prefix/opt/ffmpeg/lib/libavdevice.62.dylib", None),
        ("/usr/local/share/ffmpeg/presets/x.ffpreset", None),
    ];
    for (path, expected) in cases {
        assert_eq!(
            fix_version_segment(path, "ffmpeg", "8.0.1_2").as_deref(),
            expected,
            "path: {path}"
        );
    }
}

#[test]
fn fix_version_segment_leaves_opt_install_names_alone() {
    // Homebrew's libyaml bottle: `@@HOMEBREW_PREFIX@@/opt/libyaml/lib/libyaml-0.2.dylib`.
    assert_eq!(
        fix_version_segment(
            "/usr/local/opt/libyaml/lib/libyaml-0.2.dylib",
            "libyaml",
            "0.2.5"
        ),
        None
    );
}

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
