//! Archive fixtures shared by extraction and storage tests.

use flate2::Compression;
use flate2::write::GzEncoder;
use std::io::Write;
use tar::{Builder, Header};

/// Gzip-compresses raw bytes (typically a tar stream).
pub(crate) fn gzip(data: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

/// `.tar.gz` of regular files given as `(path, contents, mode)`.
pub(crate) fn tar_gz(entries: &[(&str, &[u8], u32)]) -> Vec<u8> {
    let mut builder = Builder::new(Vec::new());
    for &(path, content, mode) in entries {
        let mut header = Header::new_gnu();
        header.set_path(path).unwrap();
        header.set_size(content.len() as u64);
        header.set_mode(mode);
        header.set_cksum();
        builder.append(&header, content).unwrap();
    }
    gzip(&builder.into_inner().unwrap())
}
