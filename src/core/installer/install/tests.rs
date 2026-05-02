use super::cask_ops::write_json_pretty;
use super::*;
use std::fs;
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

pub(super) fn create_bottle_tarball(formula_name: &str) -> Vec<u8> {
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::Write;
    use tar::Builder;

    let mut builder = Builder::new(Vec::new());

    let mut header = tar::Header::new_gnu();
    header
        .set_path(format!("{}/1.0.0/bin/{}", formula_name, formula_name))
        .unwrap();
    header.set_size(20);
    header.set_mode(0o755);
    header.set_cksum();

    let content = format!("#!/bin/sh\necho {}", formula_name);
    builder.append(&header, content.as_bytes()).unwrap();

    let tar_data = builder.into_inner().unwrap();

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&tar_data).unwrap();
    encoder.finish().unwrap()
}

pub(super) fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    crate::core::checksum::finalize_sha256_hex(hasher)
}

pub(super) fn get_test_bottle_tag() -> String {
    crate::types::formula::bottle::current_platform_bottle_tag()
        .unwrap_or_else(|| "all".to_string())
}

pub(super) fn new_test_installer(api_client: ApiClient, tmp: &TempDir) -> Installer {
    let root = tmp.path().join("upkg");
    let prefix = tmp.path().join("homebrew");
    let blob_cache = BlobCache::new(&root.join("cache")).unwrap();
    let store = Store::new(&root).unwrap();
    let cellar = Cellar::new(&root).unwrap();
    let linker = Linker::new(&prefix).unwrap();

    Installer::new(api_client, blob_cache, store, cellar, linker, prefix)
}

#[path = "tests/auto_targets.rs"]
mod auto_targets;
#[path = "tests/casks.rs"]
mod casks;
#[path = "tests/dependencies.rs"]
mod dependencies;
#[path = "tests/helpers.rs"]
mod helpers;
#[path = "tests/lifecycle.rs"]
mod lifecycle;
#[path = "tests/planning.rs"]
mod planning;
#[path = "tests/resilience.rs"]
mod resilience;
