use std::path::{Path, PathBuf};

pub fn find_ca_bundle_from_prefix(prefix: &Path) -> Option<PathBuf> {
    let candidates = [
        prefix.join("opt/ca-certificates/share/ca-certificates/cacert.pem"),
        prefix.join("etc/ca-certificates/cacert.pem"),
        prefix.join("etc/openssl/cert.pem"),
        prefix.join("share/ca-certificates/cacert.pem"),
    ];

    candidates.into_iter().find(|p| p.exists())
}

pub fn find_ca_dir(prefix: &Path) -> Option<PathBuf> {
    let candidates = [
        prefix.join("etc/ca-certificates"),
        prefix.join("etc/openssl/certs"),
        prefix.join("share/ca-certificates"),
    ];

    candidates.into_iter().find(|p| p.exists() && p.is_dir())
}

#[cfg(all(test, target_os = "macos"))]
#[path = "ssl/tests.rs"]
mod tests;
