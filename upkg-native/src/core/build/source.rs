use std::path::{Path, PathBuf};

use crate::types::Error;
use tokio::fs;

use crate::core::checksum::verify_sha256_bytes;
use crate::core::extraction::extract_tarball;
use rama::http::{
    Response, body::util::BodyExt, client::EasyHttpWebClient, header::LOCATION,
    service::client::HttpClientExt,
};

const MAX_REDIRECTS: usize = 10;

pub async fn download_and_extract_source(
    url: &str,
    expected_checksum: Option<&str>,
    work_dir: &Path,
) -> Result<PathBuf, Error> {
    let tarball_path = work_dir.join("source.tar.gz");
    download_source(url, &tarball_path).await?;

    verify_checksum(&tarball_path, expected_checksum, url).await?;

    let src_dir = work_dir.join("src");
    fs::create_dir_all(&src_dir)
        .await
        .map_err(|e| Error::FileError {
            message: format!("failed to create source directory: {e}"),
        })?;

    if !looks_like_archive(url) {
        let filename = source_filename(url)?;
        fs::copy(&tarball_path, src_dir.join(filename))
            .await
            .map_err(|e| Error::FileError {
                message: format!("failed to stage source file: {e}"),
            })?;
        return Ok(src_dir);
    }

    extract_tarball(&tarball_path, &src_dir)?;

    find_source_root(&src_dir).await
}

fn looks_like_archive(url: &str) -> bool {
    let path = url::Url::parse(url)
        .ok()
        .map(|url| url.path().to_string())
        .unwrap_or_else(|| url.to_string());

    [
        ".tar.gz", ".tgz", ".tar.xz", ".txz", ".tar.bz2", ".tbz2", ".tar", ".zip",
    ]
    .iter()
    .any(|suffix| path.ends_with(suffix))
}

fn source_filename(url: &str) -> Result<String, Error> {
    let parsed = url::Url::parse(url).map_err(|e| Error::NetworkFailure {
        message: format!("invalid source URL '{url}': {e}"),
    })?;
    parsed
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .filter(|name| !name.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| Error::NetworkFailure {
            message: format!("source URL '{url}' does not include a filename"),
        })
}

async fn download_source(url: &str, dest: &Path) -> Result<(), Error> {
    let response = send_get_with_redirects(url).await?;

    let status = response.status();
    if !status.is_success() {
        return Err(Error::NetworkFailure {
            message: format!("source download returned HTTP {status}"),
        });
    }

    let body = response
        .into_body()
        .collect()
        .await
        .map_err(|e| Error::NetworkFailure {
            message: format!("failed to read source response: {e}"),
        })?
        .to_bytes();

    fs::write(dest, &body).await.map_err(|e| Error::FileError {
        message: format!("failed to write source tarball: {e}"),
    })
}

async fn send_get_with_redirects(url: &str) -> Result<Response, Error> {
    let client = EasyHttpWebClient::default();
    let mut current_url = url.to_string();
    let mut redirects = 0usize;

    loop {
        let response =
            client
                .get(&current_url)
                .send()
                .await
                .map_err(|e| Error::NetworkFailure {
                    message: format!("failed to download source: {e}"),
                })?;

        if !response.status().is_redirection() {
            return Ok(response);
        }

        let location = response
            .headers()
            .get(LOCATION)
            .ok_or_else(|| Error::NetworkFailure {
                message: format!("redirect ({}) without Location header", response.status()),
            })?
            .to_str()
            .map_err(|_| Error::NetworkFailure {
                message: "redirect Location header contains invalid characters".to_string(),
            })?;

        redirects += 1;
        if redirects > MAX_REDIRECTS {
            return Err(Error::NetworkFailure {
                message: format!("too many redirects while fetching {url}"),
            });
        }

        current_url = resolve_redirect_url(&current_url, location)?;
    }
}

fn resolve_redirect_url(current_url: &str, location: &str) -> Result<String, Error> {
    let base = url::Url::parse(current_url).map_err(|e| Error::NetworkFailure {
        message: format!("invalid redirect base URL '{current_url}': {e}"),
    })?;
    let next = base.join(location).map_err(|e| Error::NetworkFailure {
        message: format!("invalid redirect location '{location}': {e}"),
    })?;
    Ok(next.into())
}

async fn verify_checksum(path: &Path, expected: Option<&str>, url: &str) -> Result<(), Error> {
    let bytes = fs::read(path).await.map_err(|e| Error::FileError {
        message: format!("failed to read tarball for checksum: {e}"),
    })?;

    verify_sha256_bytes(&bytes, expected).map_err(|e| match e {
        Error::ChecksumMismatch { .. } => e,
        Error::InvalidArgument { message } => Error::InvalidArgument {
            message: format!("invalid source checksum for '{url}': {message}"),
        },
        other => other,
    })
}

async fn find_source_root(src_dir: &Path) -> Result<PathBuf, Error> {
    let mut entries = fs::read_dir(src_dir).await.map_err(|e| Error::FileError {
        message: format!("failed to read source directory: {e}"),
    })?;

    let mut subdirs = Vec::new();
    let mut has_files = false;

    while let Some(entry) = entries.next_entry().await.map_err(|e| Error::FileError {
        message: format!("failed to read directory entry: {e}"),
    })? {
        let ft = entry.file_type().await.map_err(|e| Error::FileError {
            message: format!("failed to get file type: {e}"),
        })?;
        if ft.is_dir() {
            subdirs.push(entry.path());
        } else {
            has_files = true;
        }
    }

    if subdirs.len() == 1 && !has_files {
        return Ok(subdirs.into_iter().next().unwrap());
    }

    Ok(src_dir.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn detects_archive_urls() {
        assert!(looks_like_archive("https://example.com/foo.tar.gz"));
        assert!(looks_like_archive("https://example.com/foo.zip?download=1"));
        assert!(!looks_like_archive("https://example.com/safehouse.sh"));
    }

    #[test]
    fn extracts_source_filename_from_url() {
        assert_eq!(
            source_filename("https://example.com/releases/download/v1/safehouse.sh").unwrap(),
            "safehouse.sh"
        );
    }

    #[tokio::test]
    async fn stages_single_file_sources_without_extracting() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/safehouse.sh"))
            .respond_with(ResponseTemplate::new(200).set_body_string("#!/bin/sh\n"))
            .mount(&server)
            .await;

        let work_dir = tempfile::tempdir().unwrap();
        let source_root = download_and_extract_source(
            &format!("{}/safehouse.sh", server.uri()),
            None,
            work_dir.path(),
        )
        .await
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(source_root.join("safehouse.sh")).unwrap(),
            "#!/bin/sh\n"
        );
    }
}
