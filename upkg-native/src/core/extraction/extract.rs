use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use flate2::read::GzDecoder;
use tar::Archive;
use xz2::read::XzDecoder;
use zstd::stream::read::Decoder as ZstdDecoder;

use crate::types::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompressionFormat {
    Gzip,
    Xz,
    Zstd,
    Zip,
    Unknown,
}

fn detect_compression(path: &Path) -> Result<CompressionFormat, Error> {
    let mut file = File::open(path).map_err(|e| Error::StoreCorruption {
        message: format!("failed to open tarball: {e}"),
    })?;

    let mut magic = [0u8; 6];
    let bytes_read = file.read(&mut magic).map_err(|e| Error::StoreCorruption {
        message: format!("failed to read magic bytes: {e}"),
    })?;

    if bytes_read < 2 {
        return Ok(CompressionFormat::Unknown);
    }

    if magic[0] == 0x1f && magic[1] == 0x8b {
        return Ok(CompressionFormat::Gzip);
    }

    if bytes_read >= 6 && magic[0..6] == [0xfd, 0x37, 0x7a, 0x58, 0x5a, 0x00] {
        return Ok(CompressionFormat::Xz);
    }

    if bytes_read >= 4 && magic[0..4] == [0x28, 0xb5, 0x2f, 0xfd] {
        return Ok(CompressionFormat::Zstd);
    }

    if bytes_read >= 4 && magic[0..4] == [0x50, 0x4b, 0x03, 0x04] {
        return Ok(CompressionFormat::Zip);
    }

    Ok(CompressionFormat::Unknown)
}

pub fn extract_tarball(tarball_path: &Path, dest_dir: &Path) -> Result<(), Error> {
    extract_archive(tarball_path, dest_dir)
}

pub fn extract_archive(archive_path: &Path, dest_dir: &Path) -> Result<(), Error> {
    let format = detect_compression(archive_path)?;

    let file = File::open(archive_path).map_err(|e| Error::StoreCorruption {
        message: format!("failed to open archive: {e}"),
    })?;
    let reader = BufReader::new(file);

    match format {
        CompressionFormat::Gzip => {
            let decoder = GzDecoder::new(reader);
            extract_tar_archive(decoder, dest_dir)
        }
        CompressionFormat::Xz => {
            let decoder = XzDecoder::new(reader);
            extract_tar_archive(decoder, dest_dir)
        }
        CompressionFormat::Zstd => {
            let decoder = ZstdDecoder::new(reader).map_err(|e| Error::StoreCorruption {
                message: format!("failed to create zstd decoder: {e}"),
            })?;
            extract_tar_archive(decoder, dest_dir)
        }
        CompressionFormat::Zip => extract_zip_archive(archive_path, dest_dir),
        CompressionFormat::Unknown => {
            let decoder = GzDecoder::new(reader);
            extract_tar_archive(decoder, dest_dir)
        }
    }
}

fn extract_tar_archive<R: Read>(reader: R, dest_dir: &Path) -> Result<(), Error> {
    let mut archive = Archive::new(reader);

    archive.set_preserve_permissions(true);
    archive.set_unpack_xattrs(true);

    for entry in archive.entries().map_err(|e| Error::StoreCorruption {
        message: format!("failed to read archive entries: {e}"),
    })? {
        let mut entry = entry.map_err(|e| Error::StoreCorruption {
            message: format!("failed to read archive entry: {e}"),
        })?;

        let entry_path = entry.path().map_err(|e| Error::StoreCorruption {
            message: format!("failed to read entry path: {e}"),
        })?;

        let path_display = entry_path.display().to_string();

        validate_path(&entry_path, dest_dir)?;

        entry
            .unpack_in(dest_dir)
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to unpack entry {path_display}: {e}"),
            })?;
    }

    Ok(())
}

fn extract_zip_archive(path: &Path, dest_dir: &Path) -> Result<(), Error> {
    let file = File::open(path).map_err(|e| Error::StoreCorruption {
        message: format!("failed to open zip archive: {e}"),
    })?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| Error::StoreCorruption {
        message: format!("failed to open zip archive: {e}"),
    })?;

    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| Error::StoreCorruption {
            message: format!("failed to read zip entry: {e}"),
        })?;
        let Some(raw_path) = entry.enclosed_name().map(|p| p.to_path_buf()) else {
            return Err(Error::StoreCorruption {
                message: "zip entry with invalid path".to_string(),
            });
        };

        validate_path(&raw_path, dest_dir)?;

        let out_path = dest_dir.join(&raw_path);

        if entry.is_dir() {
            std::fs::create_dir_all(&out_path).map_err(|e| Error::StoreCorruption {
                message: format!("failed to create output directory: {e}"),
            })?;
            continue;
        }

        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::StoreCorruption {
                message: format!("failed to create output parent directory: {e}"),
            })?;
        }

        let mut output = File::create(&out_path).map_err(|e| Error::StoreCorruption {
            message: format!("failed to create extracted file: {e}"),
        })?;
        std::io::copy(&mut entry, &mut output).map_err(|e| Error::StoreCorruption {
            message: format!("failed to extract zip entry: {e}"),
        })?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Some(mode) = entry.unix_mode() {
                let perms = std::fs::Permissions::from_mode(mode);
                std::fs::set_permissions(&out_path, perms).map_err(|e| Error::StoreCorruption {
                    message: format!("failed to set zip file permissions: {e}"),
                })?;
            }
        }
    }

    Ok(())
}

fn validate_path(path: &Path, dest_dir: &Path) -> Result<(), Error> {
    if path.is_absolute() {
        return Err(Error::StoreCorruption {
            message: format!("absolute path in archive: {}", path.display()),
        });
    }

    for component in path.components() {
        if let std::path::Component::ParentDir = component {
            return Err(Error::StoreCorruption {
                message: format!("path traversal in archive: {}", path.display()),
            });
        }
    }

    let full_path = dest_dir.join(path);
    let normalized = normalize_path(&full_path);

    let normalized_dest = normalize_path(dest_dir);

    if !normalized.starts_with(&normalized_dest) {
        return Err(Error::StoreCorruption {
            message: format!(
                "path escapes destination directory: {} (normalized: {}) not within {}",
                path.display(),
                normalized.display(),
                normalized_dest.display()
            ),
        });
    }

    Ok(())
}

fn normalize_path(path: &Path) -> PathBuf {
    use std::path::Component;

    let mut components = Vec::new();
    let mut is_absolute = false;

    for component in path.components() {
        match component {
            Component::RootDir => {
                is_absolute = true;
                components.push(component);
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if !components.is_empty() {
                    let last = components.last();
                    if matches!(last, Some(Component::Normal(_))) {
                        components.pop();
                    } else if matches!(last, Some(Component::RootDir)) {
                    } else {
                        components.push(component);
                    }
                } else if !is_absolute {
                    components.push(component);
                }
            }
            _ => {
                components.push(component);
            }
        }
    }

    components.iter().collect()
}

pub fn extract_tarball_from_reader<R: Read>(reader: R, dest_dir: &Path) -> Result<(), Error> {
    let decoder = GzDecoder::new(reader);
    extract_tar_archive(decoder, dest_dir)
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::fs;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use tar::Builder;
    use tempfile::TempDir;

    fn create_test_tarball(entries: Vec<(&str, &[u8], Option<u32>)>) -> Vec<u8> {
        let mut builder = Builder::new(Vec::new());

        for (path, content, mode) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_path(path).unwrap();
            header.set_size(content.len() as u64);
            header.set_mode(mode.unwrap_or(0o644));
            header.set_cksum();
            builder.append(&header, content).unwrap();
        }

        let tar_data = builder.into_inner().unwrap();

        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&tar_data).unwrap();
        encoder.finish().unwrap()
    }

    fn create_tarball_with_symlink(name: &str, target: &str) -> Vec<u8> {
        let mut builder = Builder::new(Vec::new());

        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_path(name).unwrap();
        header.set_size(0);
        header.set_mode(0o777);
        header.set_cksum();

        builder.append_link(&mut header, name, target).unwrap();

        let tar_data = builder.into_inner().unwrap();

        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&tar_data).unwrap();
        encoder.finish().unwrap()
    }

    fn create_test_zip(entries: Vec<(&str, &[u8])>) -> Vec<u8> {
        use zip::write::SimpleFileOptions;

        let cursor = std::io::Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(cursor);

        for (path, content) in entries {
            zip.start_file(path, SimpleFileOptions::default()).unwrap();
            zip.write_all(content).unwrap();
        }

        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn extracts_file_with_content() {
        let tmp = TempDir::new().unwrap();
        let tarball = create_test_tarball(vec![("hello.txt", b"Hello, World!", None)]);

        let tarball_path = tmp.path().join("test.tar.gz");
        fs::write(&tarball_path, &tarball).unwrap();

        let dest = tmp.path().join("extracted");
        fs::create_dir(&dest).unwrap();

        extract_tarball(&tarball_path, &dest).unwrap();

        let content = fs::read_to_string(dest.join("hello.txt")).unwrap();
        assert_eq!(content, "Hello, World!");
    }

    #[test]
    fn extracts_zip_file_with_content() {
        let tmp = TempDir::new().unwrap();
        let zip_data = create_test_zip(vec![("op", b"#!/bin/sh\necho op")]);

        let zip_path = tmp.path().join("test.zip");
        fs::write(&zip_path, &zip_data).unwrap();

        let dest = tmp.path().join("extracted");
        fs::create_dir(&dest).unwrap();

        extract_archive(&zip_path, &dest).unwrap();

        let content = fs::read_to_string(dest.join("op")).unwrap();
        assert_eq!(content, "#!/bin/sh\necho op");
    }

    #[test]
    fn preserves_executable_bit() {
        let tmp = TempDir::new().unwrap();
        let tarball = create_test_tarball(vec![("script.sh", b"#!/bin/sh\necho hi", Some(0o755))]);

        let tarball_path = tmp.path().join("test.tar.gz");
        fs::write(&tarball_path, &tarball).unwrap();

        let dest = tmp.path().join("extracted");
        fs::create_dir(&dest).unwrap();

        extract_tarball(&tarball_path, &dest).unwrap();

        let metadata = fs::metadata(dest.join("script.sh")).unwrap();
        let mode = metadata.permissions().mode();
        assert!(
            mode & 0o111 != 0,
            "executable bit not preserved: {:o}",
            mode
        );
    }

    #[test]
    fn preserves_symlink() {
        let tmp = TempDir::new().unwrap();
        let tarball = create_tarball_with_symlink("link", "target.txt");

        let tarball_path = tmp.path().join("test.tar.gz");
        fs::write(&tarball_path, &tarball).unwrap();

        let dest = tmp.path().join("extracted");
        fs::create_dir(&dest).unwrap();

        extract_tarball(&tarball_path, &dest).unwrap();

        let link_path = dest.join("link");
        assert!(
            link_path
                .symlink_metadata()
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read_link(&link_path).unwrap(),
            PathBuf::from("target.txt")
        );
    }

    fn create_malicious_tarball(path: &[u8]) -> Vec<u8> {
        let mut tar_data = vec![0u8; 512 + 512]; // header + one block of data

        let path_len = path.len().min(100);
        tar_data[..path_len].copy_from_slice(&path[..path_len]);

        tar_data[100..108].copy_from_slice(b"0000644\0");

        tar_data[108..116].copy_from_slice(b"0000000\0");

        tar_data[116..124].copy_from_slice(b"0000000\0");

        tar_data[124..136].copy_from_slice(b"00000000004\0");

        tar_data[136..148].copy_from_slice(b"00000000000\0");

        tar_data[156] = b'0';

        tar_data[148..156].copy_from_slice(b"        ");

        let checksum: u32 = tar_data[..512].iter().map(|&b| b as u32).sum();
        let checksum_str = format!("{:06o}\0 ", checksum);
        tar_data[148..156].copy_from_slice(checksum_str.as_bytes());

        tar_data[512..516].copy_from_slice(b"evil");

        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&tar_data).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn rejects_path_traversal() {
        let tmp = TempDir::new().unwrap();

        let tarball = create_malicious_tarball(b"../evil.txt");

        let tarball_path = tmp.path().join("evil.tar.gz");
        fs::write(&tarball_path, &tarball).unwrap();

        let dest = tmp.path().join("extracted");
        fs::create_dir(&dest).unwrap();

        let result = extract_tarball(&tarball_path, &dest);
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert!(err.to_string().contains("path traversal"));
    }

    #[test]
    fn rejects_absolute_path() {
        let tmp = TempDir::new().unwrap();

        let tarball = create_malicious_tarball(b"/etc/passwd");

        let tarball_path = tmp.path().join("absolute.tar.gz");
        fs::write(&tarball_path, &tarball).unwrap();

        let dest = tmp.path().join("extracted");
        fs::create_dir(&dest).unwrap();

        let result = extract_tarball(&tarball_path, &dest);
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert!(err.to_string().contains("absolute path"));
    }

    use rstest::rstest;

    #[rstest]
    #[case("/foo/./bar/./baz", "/foo/bar/baz", "removes dot components")]
    #[case("/foo/bar/../baz", "/foo/baz", "resolves parent dirs")]
    #[case("/foo/bar/qux/../../baz", "/foo/baz", "handles multiple parent dirs")]
    #[case(
        "../foo/bar",
        "../foo/bar",
        "preserves leading parent dirs in relative paths"
    )]
    #[case(
        "foo/./bar/../baz/./qux",
        "foo/baz/qux",
        "handles complex relative path"
    )]
    #[case("/foo/../../etc/passwd", "/etc/passwd", "cannot escape root")]
    #[case(
        "/../../../../etc/passwd",
        "/etc/passwd",
        "multiple attempts to escape root"
    )]
    #[case("/../..", "/", "root with only parent dirs")]
    fn path_normalization(#[case] input: &str, #[case] expected: &str, #[case] _description: &str) {
        let normalized = normalize_path(&PathBuf::from(input));
        assert_eq!(normalized, PathBuf::from(expected));
    }

    #[test]
    fn validate_path_rejects_normalized_escape() {
        let tmp = TempDir::new().unwrap();
        let dest = tmp.path().join("extracted");
        fs::create_dir(&dest).unwrap();

        let tricky_path = PathBuf::from("foo/../../etc/passwd");

        let result = validate_path(&tricky_path, &dest);
        assert!(result.is_err());
    }

    #[test]
    fn validate_path_accepts_safe_nested_paths() {
        let tmp = TempDir::new().unwrap();
        let dest = tmp.path().join("extracted");
        fs::create_dir(&dest).unwrap();

        let safe_path = PathBuf::from("foo/bar/baz.txt");
        let result = validate_path(&safe_path, &dest);
        assert!(result.is_ok());
    }

    #[test]
    fn validate_path_accepts_paths_with_dots_in_names() {
        let tmp = TempDir::new().unwrap();
        let dest = tmp.path().join("extracted");
        fs::create_dir(&dest).unwrap();

        let safe_path = PathBuf::from("foo/file.tar.gz");
        let result = validate_path(&safe_path, &dest);
        assert!(result.is_ok());
    }
}
