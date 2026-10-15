use super::cask_ops::write_json_pretty;
use super::*;
use std::assert_matches;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Respond, ResponseTemplate};

/// Gzipped tarball holding `files` as (path, content) pairs, each mode 0755.
pub(super) fn tar_gz<P: AsRef<std::path::Path>, C: AsRef<[u8]>>(
    files: impl IntoIterator<Item = (P, C)>,
) -> Vec<u8> {
    use flate2::{Compression, write::GzEncoder};
    use std::io::Write;

    let mut archive = tar::Builder::new(Vec::new());
    for (file, content) in files {
        let content = content.as_ref();
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        archive.append_data(&mut header, file, content).unwrap();
    }
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&archive.into_inner().unwrap()).unwrap();
    encoder.finish().unwrap()
}

/// Bottle for `formula_name` 1.0.0 holding a single `bin/<formula_name>` script.
pub(super) fn create_bottle_tarball(formula_name: &str) -> Vec<u8> {
    tar_gz([(
        format!("{formula_name}/1.0.0/bin/{formula_name}"),
        format!("#!/bin/sh\necho {formula_name}"),
    )])
}

pub(super) use crate::checksum::sha256_hex_bytes;

pub(super) fn get_test_bottle_tag() -> String {
    crate::types::formula::bottle::current_platform_bottle_tag()
        .unwrap_or_else(|| "all".to_string())
}

pub(super) struct TestInstallContext {
    pub root: PathBuf,
    pub prefix: PathBuf,
    pub installer: Installer,
}

pub(super) fn new_test_installer(api_client: ApiClient, tmp: &TempDir) -> Installer {
    new_test_context(api_client, tmp).installer
}

pub(super) fn new_test_context(api_client: ApiClient, tmp: &TempDir) -> TestInstallContext {
    let root = tmp.path().join("upkg");
    let prefix = tmp.path().join("homebrew");
    let blob_cache = BlobCache::new(&root.join("cache")).unwrap();
    let store = Store::new(&root).unwrap();
    let cellar = Cellar::new(&root).unwrap();
    let linker = Linker::new(&prefix).unwrap();

    let installer = Installer::new(
        api_client,
        blob_cache,
        store,
        cellar,
        linker,
        prefix.clone(),
    );

    TestInstallContext {
        root,
        prefix,
        installer,
    }
}

/// Test context whose formula API is served by `server`.
pub(super) fn server_context(server: &MockServer, tmp: &TempDir) -> TestInstallContext {
    new_test_context(ApiClient::with_base_url(server.uri()), tmp)
}

/// Installer whose formula API is served by `server`.
pub(super) fn server_installer(server: &MockServer, tmp: &TempDir) -> Installer {
    server_context(server, tmp).installer
}

// MOCKED: the helpers below answer GET `route` on a local wiremock server.

pub(super) async fn mount_response(
    server: &MockServer,
    route: &str,
    response: impl Respond + 'static,
) {
    Mock::given(method("GET"))
        .and(path(route))
        .respond_with(response)
        .mount(server)
        .await;
}

pub(super) async fn mount_text(server: &MockServer, route: &str, body: impl AsRef<str>) {
    let response = ResponseTemplate::new(200).set_body_string(body.as_ref());
    mount_response(server, route, response).await;
}

pub(super) async fn mount_json(server: &MockServer, route: &str, body: serde_json::Value) {
    mount_response(
        server,
        route,
        ResponseTemplate::new(200).set_body_json(body),
    )
    .await;
}

pub(super) async fn mount_bytes(server: &MockServer, route: &str, body: Vec<u8>) {
    mount_response(
        server,
        route,
        ResponseTemplate::new(200).set_body_bytes(body),
    )
    .await;
}

pub(super) async fn mount_status(server: &MockServer, route: &str, status: u16) {
    mount_response(server, route, ResponseTemplate::new(status)).await;
}

/// Route that [`bottled_formula_json`] points the `name`/`version` bottle at.
pub(super) fn bottle_route(name: &str, version: &str) -> String {
    format!(
        "/bottles/{name}-{version}.{}.bottle.tar.gz",
        get_test_bottle_tag()
    )
}

/// Formula JSON with one bottle for the current platform, hosted on `server`.
pub(super) fn bottled_formula_json(
    server: &MockServer,
    name: &str,
    version: &str,
    deps: &[&str],
    sha: &str,
) -> String {
    let tag = get_test_bottle_tag();
    let deps = serde_json::to_string(deps).unwrap();
    format!(
        r#"{{
                "name": "{name}",
                "versions": {{ "stable": "{version}" }},
                "dependencies": {deps},
                "bottle": {{
                    "stable": {{
                        "files": {{
                            "{tag}": {{
                                "url": "{uri}{route}",
                                "sha256": "{sha}"
                            }}
                        }}
                    }}
                }}
            }}"#,
        uri = server.uri(),
        route = bottle_route(name, version),
    )
}

/// Serves the formula JSON for `name` (bottle checksum `sha`) and answers its
/// bottle route with `bottle`.
pub(super) async fn mount_bottle_response(
    server: &MockServer,
    name: &str,
    version: &str,
    deps: &[&str],
    sha: &str,
    bottle: impl Respond + 'static,
) {
    let json = bottled_formula_json(server, name, version, deps, sha);
    mount_text(server, &format!("/{name}.json"), json).await;
    mount_response(server, &bottle_route(name, version), bottle).await;
}

/// Serves a working bottled formula `name` and returns its bottle checksum.
pub(super) async fn mount_bottled(
    server: &MockServer,
    name: &str,
    version: &str,
    deps: &[&str],
) -> String {
    let bottle = create_bottle_tarball(name);
    let sha = sha256_hex_bytes(&bottle);
    let response = ResponseTemplate::new(200).set_body_bytes(bottle);
    mount_bottle_response(server, name, version, deps, &sha, response).await;
    sha
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
#[path = "tests/post_install.rs"]
mod post_install;
#[path = "tests/reinstall.rs"]
mod reinstall;
#[path = "tests/resilience.rs"]
mod resilience;
