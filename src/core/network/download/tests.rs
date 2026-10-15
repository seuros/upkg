use super::*;
use crate::checksum::sha256_hex_bytes;
use std::assert_matches;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

// MOCKED: wiremock stands in for bottle hosts; blobs land in a TempDir cache.
const HELLO_WORLD: &[u8] = b"hello world";
const HELLO_WORLD_SHA256: &str = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";

fn downloader(tmp: &TempDir) -> Downloader {
    Downloader::new(BlobCache::new(tmp.path()).unwrap())
}

/// GET `route` answering 200 with `content`.
fn get_bytes(route: &str, content: &[u8]) -> Mock {
    Mock::given(method("GET"))
        .and(path(route))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(content.to_vec()))
}

/// HEAD `route` advertising `len`, plus `Accept-Ranges: bytes` when `ranges`.
async fn mount_head(server: &MockServer, route: &str, len: usize, ranges: bool) {
    let mut response = ResponseTemplate::new(200);
    if ranges {
        response = response.append_header("Accept-Ranges", "bytes");
    }
    Mock::given(method("HEAD"))
        .and(path(route))
        .respond_with(response.append_header("Content-Length", len.to_string()))
        .mount(server)
        .await;
}

/// Inclusive `(start, end)` from a `Range: bytes=start-end` header.
fn requested_range(req: &Request) -> Option<(usize, usize)> {
    let range = req.headers.get("Range")?.to_str().unwrap();
    let (start, end) = range
        .strip_prefix("bytes=")
        .unwrap()
        .split_once('-')
        .unwrap();
    Some((start.parse().unwrap(), end.parse().unwrap()))
}

/// 206 Partial Content for `content[start..=end]`.
fn partial_content(content: &[u8], start: usize, end: usize) -> ResponseTemplate {
    let chunk = &content[start..=end];
    ResponseTemplate::new(206)
        .append_header("Content-Length", chunk.len().to_string())
        .append_header(
            "Content-Range",
            format!("bytes {start}-{end}/{}", content.len()),
        )
        .set_body_bytes(chunk.to_vec())
}

/// Downloads `url`, asserts success, and returns the blob bytes.
async fn download_ok(downloader: &Downloader, url: &str, sha256: &str) -> Vec<u8> {
    let result = downloader.download(url, sha256).await;
    assert!(result.is_ok(), "Download failed: {:?}", result.err());
    let blob_path = result.unwrap();
    assert!(blob_path.exists());
    std::fs::read(&blob_path).unwrap()
}

#[test]
fn build_rama_client_does_not_panic() {
    let _ = http_client::build_rama_client();
}

#[tokio::test]
async fn valid_checksum_passes() {
    let mock_server = MockServer::start().await;
    get_bytes("/test.tar.gz", HELLO_WORLD)
        .mount(&mock_server)
        .await;

    let tmp = TempDir::new().unwrap();
    let url = format!("{}/test.tar.gz", mock_server.uri());
    let downloaded = download_ok(&downloader(&tmp), &url, HELLO_WORLD_SHA256).await;

    assert_eq!(downloaded, HELLO_WORLD);
}

#[tokio::test]
async fn mismatch_deletes_blob_and_errors() {
    let mock_server = MockServer::start().await;
    let wrong_sha256 = "0000000000000000000000000000000000000000000000000000000000000000";
    get_bytes("/test.tar.gz", HELLO_WORLD)
        .mount(&mock_server)
        .await;

    let tmp = TempDir::new().unwrap();
    let url = format!("{}/test.tar.gz", mock_server.uri());
    let result = downloader(&tmp).download(&url, wrong_sha256).await;

    assert_matches!(result, Err(Error::ChecksumMismatch { .. }));

    let blob_path = tmp
        .path()
        .join("blobs")
        .join(format!("{wrong_sha256}.tar.gz"));
    assert!(!blob_path.exists());

    let tmp_path = tmp
        .path()
        .join("tmp")
        .join(format!("{wrong_sha256}.tar.gz.part"));
    assert!(!tmp_path.exists());
}

#[tokio::test]
async fn skips_download_if_blob_exists() {
    let mock_server = MockServer::start().await;
    get_bytes("/test.tar.gz", HELLO_WORLD)
        .expect(0)
        .mount(&mock_server)
        .await;

    let tmp = TempDir::new().unwrap();
    let blob_cache = BlobCache::new(tmp.path()).unwrap();

    let mut writer = blob_cache.start_write(HELLO_WORLD_SHA256).unwrap();
    writer.write_all(HELLO_WORLD).unwrap();
    writer.commit().unwrap();

    let downloader = Downloader::new(blob_cache);
    let url = format!("{}/test.tar.gz", mock_server.uri());
    let result = downloader.download(&url, HELLO_WORLD_SHA256).await;

    assert!(result.is_ok());
}

#[tokio::test]
async fn peak_concurrent_downloads_within_limit() {
    let mock_server = MockServer::start().await;
    let concurrent_count = Arc::new(AtomicUsize::new(0));
    let max_concurrent = Arc::new(AtomicUsize::new(0));

    let content = b"test content";
    let count_clone = concurrent_count.clone();
    let max_clone = max_concurrent.clone();

    Mock::given(method("GET"))
        .respond_with(move |_: &Request| {
            let current = count_clone.fetch_add(1, Ordering::SeqCst) + 1;
            max_clone.fetch_max(current, Ordering::SeqCst);

            std::thread::sleep(Duration::from_millis(50));

            count_clone.fetch_sub(1, Ordering::SeqCst);
            ResponseTemplate::new(200).set_body_bytes(content.to_vec())
        })
        .mount(&mock_server)
        .await;

    let tmp = TempDir::new().unwrap();
    let blob_cache = BlobCache::new(tmp.path()).unwrap();
    let downloader = ParallelDownloader::new(blob_cache); // Uses global concurrency

    let requests: Vec<_> = (0..5)
        .map(|i| DownloadRequest {
            url: format!("{}/file{i}.tar.gz", mock_server.uri()),
            sha256: format!("{i:064x}"),
            name: format!("pkg{i}"),
        })
        .collect();

    let _ = downloader.download_all(requests).await;

    let peak = max_concurrent.load(Ordering::SeqCst);
    assert!(
        peak <= GLOBAL_DOWNLOAD_CONCURRENCY,
        "peak concurrent downloads was {peak}, expected <= {GLOBAL_DOWNLOAD_CONCURRENCY}"
    );
}

#[tokio::test]
async fn same_blob_requested_multiple_times_fetches_once() {
    let mock_server = MockServer::start().await;
    let content = b"deduplicated content";
    let actual_sha256 = sha256_hex_bytes(content);

    Mock::given(method("GET"))
        .and(path("/dedup.tar.gz"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(content.to_vec())
                .set_delay(Duration::from_millis(100)),
        )
        .expect(1) // Should only be called once
        .mount(&mock_server)
        .await;

    let tmp = TempDir::new().unwrap();
    let blob_cache = BlobCache::new(tmp.path()).unwrap();
    let downloader = ParallelDownloader::new(blob_cache);

    let requests: Vec<_> = (0..5)
        .map(|i| DownloadRequest {
            url: format!("{}/dedup.tar.gz", mock_server.uri()),
            sha256: actual_sha256.clone(),
            name: format!("dedup{i}"),
        })
        .collect();

    let results = downloader.download_all(requests).await.unwrap();

    assert_eq!(results.len(), 5);
    for path in &results {
        assert!(path.exists());
    }
}

#[tokio::test]
async fn chunked_download_for_large_files() {
    let mock_server = MockServer::start().await;
    let large_content = vec![0xABu8; 15 * 1024 * 1024];
    mount_head(&mock_server, "/large.tar.gz", large_content.len(), true).await;

    let range_requests = Arc::new(AtomicUsize::new(0));
    let range_requests_clone = range_requests.clone();
    let content = large_content.clone();

    Mock::given(method("GET"))
        .and(path("/large.tar.gz"))
        .respond_with(move |req: &Request| match requested_range(req) {
            Some((start, end)) => {
                range_requests_clone.fetch_add(1, Ordering::SeqCst);
                partial_content(&content, start, end)
            }
            None => ResponseTemplate::new(200).set_body_bytes(content.clone()),
        })
        .mount(&mock_server)
        .await;

    let tmp = TempDir::new().unwrap();
    let url = format!("{}/large.tar.gz", mock_server.uri());
    let downloaded = download_ok(&downloader(&tmp), &url, &sha256_hex_bytes(&large_content)).await;

    let range_count = range_requests.load(Ordering::SeqCst);
    assert!(
        range_count > 0,
        "Expected multiple Range requests, got {range_count}"
    );
    assert_eq!(downloaded.len(), large_content.len());
    assert_eq!(downloaded, large_content);
}

#[tokio::test]
async fn fallback_to_normal_download_when_ranges_not_supported() {
    let mock_server = MockServer::start().await;
    let large_content = vec![0xCDu8; 15 * 1024 * 1024];
    mount_head(&mock_server, "/large.tar.gz", large_content.len(), false).await;
    get_bytes("/large.tar.gz", &large_content)
        .mount(&mock_server)
        .await;

    let tmp = TempDir::new().unwrap();
    let url = format!("{}/large.tar.gz", mock_server.uri());
    let downloaded = download_ok(&downloader(&tmp), &url, &sha256_hex_bytes(&large_content)).await;

    assert_eq!(downloaded, large_content);
}

#[tokio::test]
async fn small_files_dont_use_chunked_download() {
    let mock_server = MockServer::start().await;
    let small_content = vec![0xEFu8; 1024 * 1024];
    mount_head(&mock_server, "/small.tar.gz", small_content.len(), true).await;

    let range_used = Arc::new(AtomicUsize::new(0));
    let range_used_clone = range_used.clone();
    let content = small_content.clone();

    Mock::given(method("GET"))
        .and(path("/small.tar.gz"))
        .respond_with(move |req: &Request| {
            if req.headers.get("Range").is_some() {
                range_used_clone.fetch_add(1, Ordering::SeqCst);
            }
            ResponseTemplate::new(200).set_body_bytes(content.clone())
        })
        .mount(&mock_server)
        .await;

    let tmp = TempDir::new().unwrap();
    let url = format!("{}/small.tar.gz", mock_server.uri());
    let downloaded = download_ok(&downloader(&tmp), &url, &sha256_hex_bytes(&small_content)).await;

    assert_eq!(
        range_used.load(Ordering::SeqCst),
        0,
        "Small files should not use chunked download"
    );
    assert_eq!(downloaded, small_content);
}

#[tokio::test]
async fn chunked_download_respects_concurrency_limit() {
    let mock_server = MockServer::start().await;
    let large_content = vec![0xABu8; 40 * 1024 * 1024];
    mount_head(&mock_server, "/large.tar.gz", large_content.len(), true).await;

    let concurrent_count = Arc::new(AtomicUsize::new(0));
    let max_concurrent = Arc::new(AtomicUsize::new(0));
    let concurrent_clone = concurrent_count.clone();
    let max_clone = max_concurrent.clone();
    let content = large_content.clone();

    Mock::given(method("GET"))
        .and(path("/large.tar.gz"))
        .respond_with(move |req: &Request| match requested_range(req) {
            Some((start, end)) => {
                let current = concurrent_clone.fetch_add(1, Ordering::SeqCst) + 1;
                max_clone.fetch_max(current, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(50));
                concurrent_clone.fetch_sub(1, Ordering::SeqCst);
                partial_content(&content, start, end)
            }
            None => ResponseTemplate::new(200).set_body_bytes(content.clone()),
        })
        .mount(&mock_server)
        .await;

    let tmp = TempDir::new().unwrap();
    let url = format!("{}/large.tar.gz", mock_server.uri());
    let downloaded = download_ok(&downloader(&tmp), &url, &sha256_hex_bytes(&large_content)).await;

    let peak = max_concurrent.load(Ordering::SeqCst);
    assert!(
        peak <= MAX_CONCURRENT_CHUNKS,
        "Peak concurrent downloads was {peak}, expected <= {MAX_CONCURRENT_CHUNKS}"
    );
    assert_eq!(downloaded.len(), large_content.len());
    assert_eq!(downloaded, large_content);
}

#[test]
fn extract_scope_for_url_supports_core_and_tapped_packages() {
    const CASES: &[(&str, &str)] = &[
        (
            "https://ghcr.io/v2/homebrew/core/lz4/blobs/sha256:abc",
            "repository:homebrew/core/lz4:pull",
        ),
        (
            "https://ghcr.io/v2/hashicorp/tap/terraform/blobs/sha256:abc",
            "repository:hashicorp/tap/terraform:pull",
        ),
    ];
    for (url, expected) in CASES {
        assert_eq!(
            super::extract_scope_for_url(url).as_deref(),
            Some(*expected),
            "url: {url}"
        );
    }
}

#[tokio::test]
async fn download_retries_on_transient_network_failure() {
    let mock_server = MockServer::start().await;
    let content = b"retry success";

    let attempt_count = Arc::new(AtomicUsize::new(0));
    let count_clone = attempt_count.clone();

    Mock::given(method("GET"))
        .and(path("/flaky.tar.gz"))
        .respond_with(move |_: &Request| {
            let attempt = count_clone.fetch_add(1, Ordering::SeqCst);
            if attempt < 2 {
                // Fail first 2 attempts
                ResponseTemplate::new(500).set_body_string("Internal Server Error")
            } else {
                // Succeed on 3rd attempt
                ResponseTemplate::new(200).set_body_bytes(content.to_vec())
            }
        })
        .mount(&mock_server)
        .await;

    let tmp = TempDir::new().unwrap();
    let url = format!("{}/flaky.tar.gz", mock_server.uri());
    let result = downloader(&tmp).download(&url, &sha256_hex_bytes(content)).await;

    assert!(result.is_ok(), "Download should succeed after retries");
    assert!(
        attempt_count.load(Ordering::SeqCst) >= 3,
        "Should have retried at least 3 times"
    );
}

#[tokio::test]
async fn download_follows_redirects() {
    let mock_server = MockServer::start().await;
    let content = b"redirected content";

    // Setup redirect chain: /start -> /middle -> /final
    for (from, status, to) in [
        ("/start.tar.gz", 302, "/middle.tar.gz"),
        ("/middle.tar.gz", 301, "/final.tar.gz"),
    ] {
        Mock::given(method("GET"))
            .and(path(from))
            .respond_with(
                ResponseTemplate::new(status)
                    .insert_header("Location", format!("{}{to}", mock_server.uri())),
            )
            .mount(&mock_server)
            .await;
    }
    get_bytes("/final.tar.gz", content)
        .mount(&mock_server)
        .await;

    let tmp = TempDir::new().unwrap();
    let url = format!("{}/start.tar.gz", mock_server.uri());
    let result = downloader(&tmp).download(&url, &sha256_hex_bytes(content)).await;

    assert!(result.is_ok(), "Should follow redirects");
    assert_eq!(std::fs::read(result.unwrap()).unwrap(), content);
}

#[test]
fn transform_url_to_mirror_rewrites_only_ghcr() {
    const MIRROR: &str = "mirror.example.com";
    const CASES: &[(&str, Option<&str>)] = &[
        (
            "https://ghcr.io/v2/homebrew/core/wget/blobs/sha256:abc",
            Some("https://mirror.example.com/v2/homebrew/core/wget/blobs/sha256:abc"),
        ),
        ("https://example.com/file.tar.gz", None),
    ];
    for (original, expected) in CASES {
        assert_eq!(
            super::transform_url_to_mirror(original, MIRROR).as_deref(),
            *expected,
            "url: {original}"
        );
    }
}
