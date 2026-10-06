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

#[cfg(all(test, target_os = "macos"))]
#[path = "extract/tests.rs"]
mod tests;
