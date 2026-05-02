use crate::error::UpkgError;

#[cfg(any(target_os = "macos", target_os = "linux"))]
use rama::http::BodyExtractExt;

const REPO_OWNER: &str = "seuros";
const REPO_NAME: &str = "upkg";
const MAX_REDIRECTS: usize = 10;

pub fn run(dry_run: bool) -> Result<(), UpkgError> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        run_unix(dry_run)
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = dry_run;
        Err(UpkgError::SelfUpgrade(
            "self-upgrade is not available on this platform yet".to_string(),
        ))
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn run_unix(dry_run: bool) -> Result<(), UpkgError> {
    let target = release_target().ok_or_else(|| {
        UpkgError::SelfUpgrade(format!(
            "self-upgrade is not available for {}-{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ))
    })?;

    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(run_unix_async(dry_run, target))
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
async fn run_unix_async(dry_run: bool, target: ReleaseTarget) -> Result<(), UpkgError> {
    let client = build_rama_client();
    let release = fetch_latest_release(&client).await?;
    let latest_version = version_from_tag(&release.tag_name);
    let asset_name = release_asset_name(&latest_version, target);

    let current_version = env!("CARGO_PKG_VERSION");
    if dry_run {
        println!("current: v{current_version}");
        println!("latest:  {}", release.tag_name);
        println!("asset:   {asset_name}");
        if latest_version == current_version {
            println!("status:  already up to date");
        } else {
            println!("status:  would install v{latest_version}");
        }
        return Ok(());
    }

    if latest_version == current_version {
        println!("upkg is already up to date at v{current_version}");
        return Ok(());
    }

    let asset = release.asset(&asset_name).ok_or_else(|| {
        UpkgError::SelfUpgrade(format!(
            "release '{}' does not include asset '{}'",
            release.tag_name, asset_name
        ))
    })?;
    let checksums = release.asset("SHA256SUMS.txt").ok_or_else(|| {
        UpkgError::SelfUpgrade(format!(
            "release '{}' does not include SHA256SUMS.txt",
            release.tag_name
        ))
    })?;
    let checksums_body = fetch_text(&client, &checksums.browser_download_url).await?;
    let sha256 = checksum_for_asset(&checksums_body, &asset_name)?;

    let work_dir = create_work_dir()?;
    let archive_path = work_dir.join(&asset_name);
    download_to_file(&client, &asset.browser_download_url, &archive_path, &sha256).await?;

    let extract_dir = work_dir.join("extract");
    std::fs::create_dir_all(&extract_dir)?;
    extract_tar_gz(&archive_path, &extract_dir)?;

    let extracted_binary = extract_dir.join(binary_name());
    if !extracted_binary.exists() {
        return Err(UpkgError::SelfUpgrade(format!(
            "release asset '{}' did not contain '{}'",
            asset_name,
            binary_name()
        )));
    }

    replace_current_binary(&extracted_binary)?;
    let _ = std::fs::remove_dir_all(&work_dir);
    println!("Updated upkg to v{latest_version}");
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn build_rama_client()
-> rama::service::BoxService<rama::http::Request, rama::http::Response, rama::error::OpaqueError> {
    use rama::Service;
    use rama::http::{Body, client::EasyHttpWebClient, client::HttpClientService};
    use rama::net::client::pool::http::HttpPooledConnectorConfig;

    EasyHttpWebClient::connector_builder()
        .with_default_transport_connector()
        .without_tls_proxy_support()
        .with_proxy_support()
        .with_tls_support_using_rustls(None)
        .with_default_http_connector()
        .try_with_connection_pool::<HttpClientService<Body>>(HttpPooledConnectorConfig::default())
        .expect("failed to build HTTP client with connection pool")
        .build_client()
        .boxed()
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[derive(Debug, serde::Deserialize)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubAsset>,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl GitHubRelease {
    fn asset(&self, name: &str) -> Option<&GitHubAsset> {
        self.assets.iter().find(|asset| asset.name == name)
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[derive(Debug, serde::Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
async fn fetch_latest_release(
    client: &rama::service::BoxService<
        rama::http::Request,
        rama::http::Response,
        rama::error::OpaqueError,
    >,
) -> Result<GitHubRelease, UpkgError> {
    let url = format!("https://api.github.com/repos/{REPO_OWNER}/{REPO_NAME}/releases/latest");
    let response = send_get_with_redirects(client, &url).await?;
    response
        .try_into_json::<GitHubRelease>()
        .await
        .map_err(|e| UpkgError::SelfUpgrade(format!("failed to parse GitHub release JSON: {e}")))
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
async fn fetch_text(
    client: &rama::service::BoxService<
        rama::http::Request,
        rama::http::Response,
        rama::error::OpaqueError,
    >,
    url: &str,
) -> Result<String, UpkgError> {
    let response = send_get_with_redirects(client, url).await?;
    response
        .try_into_string()
        .await
        .map_err(|e| UpkgError::SelfUpgrade(format!("failed to read response body: {e}")))
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
async fn send_get_with_redirects(
    client: &rama::service::BoxService<
        rama::http::Request,
        rama::http::Response,
        rama::error::OpaqueError,
    >,
    url: &str,
) -> Result<rama::http::Response, UpkgError> {
    use rama::http::StatusCode;
    use rama::http::service::client::HttpClientExt;

    let mut current_url = url.to_string();
    let mut redirects = 0usize;

    loop {
        let response = client
            .get(&current_url)
            .header("User-Agent", "upkg")
            .send()
            .await
            .map_err(|e| UpkgError::SelfUpgrade(format!("request failed: {e}")))?;

        if response.status().is_redirection() {
            redirects += 1;
            if redirects > MAX_REDIRECTS {
                return Err(UpkgError::SelfUpgrade(format!(
                    "too many redirects while fetching {url}"
                )));
            }
            let Some(location) = response.headers().get("Location") else {
                return Err(UpkgError::SelfUpgrade(format!(
                    "redirect ({}) without Location header",
                    response.status()
                )));
            };
            let location = location.to_str().map_err(|_| {
                UpkgError::SelfUpgrade("redirect Location header is not valid UTF-8".to_string())
            })?;
            current_url = resolve_redirect_url(&current_url, location)?;
            continue;
        }

        if response.status() != StatusCode::OK {
            return Err(UpkgError::SelfUpgrade(format!(
                "request returned HTTP {}",
                response.status()
            )));
        }

        return Ok(response);
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn resolve_redirect_url(current_url: &str, location: &str) -> Result<String, UpkgError> {
    if location.starts_with("http://") || location.starts_with("https://") {
        return Ok(location.to_string());
    }

    let base = url::Url::parse(current_url)
        .map_err(|e| UpkgError::SelfUpgrade(format!("invalid redirect base URL: {e}")))?;
    base.join(location)
        .map(|url| url.to_string())
        .map_err(|e| UpkgError::SelfUpgrade(format!("invalid redirect location: {e}")))
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
async fn download_to_file(
    client: &rama::service::BoxService<
        rama::http::Request,
        rama::http::Response,
        rama::error::OpaqueError,
    >,
    url: &str,
    path: &std::path::Path,
    expected_sha256: &str,
) -> Result<(), UpkgError> {
    use sha2::{Digest, Sha256};
    use std::io::Write;

    let response = send_get_with_redirects(client, url).await?;
    let mut body = response.into_body();
    let mut file = std::fs::File::create(path)?;
    let mut hasher = Sha256::new();

    while let Some(chunk) = body.chunk().await.map_err(|e| {
        UpkgError::SelfUpgrade(format!("failed to read release asset response: {e}"))
    })? {
        hasher.update(&chunk);
        file.write_all(&chunk)?;
    }

    let actual = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if actual != expected_sha256 {
        let _ = std::fs::remove_file(path);
        return Err(UpkgError::SelfUpgrade(format!(
            "checksum mismatch for downloaded release asset: expected {expected_sha256}, got {actual}"
        )));
    }

    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn extract_tar_gz(
    archive_path: &std::path::Path,
    dest_dir: &std::path::Path,
) -> Result<(), UpkgError> {
    let file = std::fs::File::open(archive_path)?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    archive.unpack(dest_dir).map_err(|e| {
        UpkgError::SelfUpgrade(format!(
            "failed to extract release archive '{}': {e}",
            archive_path.display()
        ))
    })
}

fn checksum_for_asset(checksums: &str, asset_name: &str) -> Result<String, UpkgError> {
    for line in checksums.lines() {
        let mut parts = line.split_whitespace();
        let Some(hash) = parts.next() else {
            continue;
        };
        let Some(name) = parts.next() else {
            continue;
        };
        if name.trim_start_matches('*') == asset_name {
            return Ok(hash.to_string());
        }
    }

    Err(UpkgError::SelfUpgrade(format!(
        "SHA256SUMS.txt does not include {asset_name}"
    )))
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn create_work_dir() -> Result<std::path::PathBuf, UpkgError> {
    let dir = std::env::temp_dir().join(format!(
        "upkg-self-upgrade-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn replace_current_binary(extracted_binary: &std::path::Path) -> Result<(), UpkgError> {
    let current = std::env::current_exe()?;
    let parent = current.parent().ok_or_else(|| {
        UpkgError::SelfUpgrade(format!(
            "current executable '{}' has no parent",
            current.display()
        ))
    })?;
    let replacement = parent.join(format!(".upkg-update-{}", std::process::id()));

    std::fs::copy(extracted_binary, &replacement)?;
    let metadata = std::fs::metadata(extracted_binary)?;
    std::fs::set_permissions(&replacement, metadata.permissions())?;
    std::fs::rename(&replacement, &current).map_err(|e| {
        let _ = std::fs::remove_file(&replacement);
        UpkgError::SelfUpgrade(format!("failed to replace '{}': {e}", current.display()))
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReleaseTarget {
    triple: &'static str,
    archive: ArchiveKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArchiveKind {
    TarGz,
    #[allow(dead_code)]
    Zip,
}

fn release_target() -> Option<ReleaseTarget> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        return Some(ReleaseTarget {
            triple: "aarch64-apple-darwin",
            archive: ArchiveKind::TarGz,
        });
    }

    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        return Some(ReleaseTarget {
            triple: "x86_64-apple-darwin",
            archive: ArchiveKind::TarGz,
        });
    }

    #[cfg(all(target_os = "linux", target_arch = "aarch64", target_env = "gnu"))]
    {
        return Some(ReleaseTarget {
            triple: "aarch64-unknown-linux-gnu",
            archive: ArchiveKind::TarGz,
        });
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
    {
        return Some(ReleaseTarget {
            triple: "x86_64-unknown-linux-gnu",
            archive: ArchiveKind::TarGz,
        });
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "musl"))]
    {
        return Some(ReleaseTarget {
            triple: "x86_64-unknown-linux-musl",
            archive: ArchiveKind::TarGz,
        });
    }

    #[allow(unreachable_code)]
    None
}

fn release_asset_name(version: &str, target: ReleaseTarget) -> String {
    let extension = match target.archive {
        ArchiveKind::TarGz => "tar.gz",
        ArchiveKind::Zip => "zip",
    };
    format!(
        "upkg-v{}-{}.{}",
        version_from_tag(version),
        target.triple,
        extension
    )
}

fn version_from_tag(tag_or_version: &str) -> String {
    tag_or_version
        .strip_prefix("upkg-v")
        .or_else(|| tag_or_version.strip_prefix('v'))
        .unwrap_or(tag_or_version)
        .to_string()
}

fn binary_name() -> &'static str {
    #[cfg(windows)]
    {
        "upkg.exe"
    }

    #[cfg(not(windows))]
    {
        "upkg"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_asset_name_uses_upkg_release_format() {
        let target = ReleaseTarget {
            triple: "aarch64-apple-darwin",
            archive: ArchiveKind::TarGz,
        };

        assert_eq!(
            release_asset_name("upkg-v1.2.3", target),
            "upkg-v1.2.3-aarch64-apple-darwin.tar.gz"
        );
    }

    #[test]
    fn version_from_tag_handles_component_tags() {
        assert_eq!(version_from_tag("upkg-v1.2.3"), "1.2.3");
        assert_eq!(version_from_tag("v1.2.3"), "1.2.3");
        assert_eq!(version_from_tag("1.2.3"), "1.2.3");
    }

    #[test]
    fn release_target_is_supported_for_current_platform_or_cleanly_missing() {
        if let Some(target) = release_target() {
            assert!(target.triple.contains(std::env::consts::ARCH));
        }
    }

    #[test]
    fn checksum_for_asset_finds_matching_line() {
        let body = "abc  other.tar.gz\ndef  upkg-v1.2.3-aarch64-apple-darwin.tar.gz\n";
        assert_eq!(
            checksum_for_asset(body, "upkg-v1.2.3-aarch64-apple-darwin.tar.gz").unwrap(),
            "def"
        );
    }
}
