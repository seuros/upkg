use super::*;
use std::assert_matches;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[test]
fn build_rama_client_does_not_panic() {
    let _ = http_client::build_rama_client();
}

#[tokio::test]
async fn valid_checksum_passes() {
    let mock_server = MockServer::start().await;
    let content = b"hello world";
    let sha256 = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";

    Mock::given(method("GET"))
        .and(path("/test.tar.gz"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(content.to_vec()))
        .mount(&mock_server)
        .await;

    let tmp = TempDir::new().unwrap();
    let blob_cache = BlobCache::new(tmp.path()).unwrap();
    let downloader = Downloader::new(blob_cache);

    let url = format!("{}/test.tar.gz", mock_server.uri());
    let result = downloader.download(&url, sha256).await;

    assert!(result.is_ok());
    let blob_path = result.unwrap();
    assert!(blob_path.exists());
    assert_eq!(std::fs::read(&blob_path).unwrap(), content);
}

#[tokio::test]
async fn mismatch_deletes_blob_and_errors() {
    let mock_server = MockServer::start().await;
    let content = b"hello world";
    let wrong_sha256 = "0000000000000000000000000000000000000000000000000000000000000000";

    Mock::given(method("GET"))
        .and(path("/test.tar.gz"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(content.to_vec()))
        .mount(&mock_server)
        .await;

    let tmp = TempDir::new().unwrap();
    let blob_cache = BlobCache::new(tmp.path()).unwrap();
    let downloader = Downloader::new(blob_cache);

    let url = format!("{}/test.tar.gz", mock_server.uri());
    let result = downloader.download(&url, wrong_sha256).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_matches!(err, Error::ChecksumMismatch { .. });

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
    let content = b"hello world";
    let sha256 = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";

    Mock::given(method("GET"))
        .and(path("/test.tar.gz"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(content.to_vec()))
        .expect(0)
        .mount(&mock_server)
        .await;

    let tmp = TempDir::new().unwrap();
    let blob_cache = BlobCache::new(tmp.path()).unwrap();

    let mut writer = blob_cache.start_write(sha256).unwrap();
    writer.write_all(content).unwrap();
    writer.commit().unwrap();

    let downloader = Downloader::new(blob_cache);
    let url = format!("{}/test.tar.gz", mock_server.uri());
    let result = downloader.download(&url, sha256).await;

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
        .respond_with(move |_: &wiremock::Request| {
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
        .map(|i| {
            let sha256 = format!("{:064x}", i);
            DownloadRequest {
                url: format!("{}/file{i}.tar.gz", mock_server.uri()),
                sha256,
                name: format!("pkg{i}"),
            }
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

    let actual_sha256 = {
        let mut hasher = Sha256::new();
        hasher.update(content);
        finalize_sha256_hex(hasher)
    };

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
    let actual_sha256 = {
        let mut hasher = Sha256::new();
        hasher.update(&large_content);
        finalize_sha256_hex(hasher)
    };

    Mock::given(method("HEAD"))
        .and(path("/large.tar.gz"))
        .respond_with(
            ResponseTemplate::new(200)
                .append_header("Accept-Ranges", "bytes")
                .append_header("Content-Length", large_content.len().to_string()),
        )
        .mount(&mock_server)
        .await;

    let range_requests = Arc::new(AtomicUsize::new(0));
    let range_requests_clone = range_requests.clone();
    let large_content_for_closure = large_content.clone();

    Mock::given(method("GET"))
        .and(path("/large.tar.gz"))
        .respond_with(move |req: &wiremock::Request| {
            if let Some(range_header) = req.headers.get("Range") {
                range_requests_clone.fetch_add(1, Ordering::SeqCst);

                let range_str = range_header.to_str().unwrap();
                let range_part = range_str.strip_prefix("bytes=").unwrap();
                let (start_str, end_str) = range_part.split_once('-').unwrap();
                let start: usize = start_str.parse().unwrap();
                let end: usize = end_str.parse().unwrap();

                let chunk = &large_content_for_closure[start..=end];
                ResponseTemplate::new(206) // 206 Partial Content
                    .append_header("Content-Length", chunk.len().to_string())
                    .append_header(
                        "Content-Range",
                        format!(
                            "bytes {}-{}/{}",
                            start,
                            end,
                            large_content_for_closure.len()
                        ),
                    )
                    .set_body_bytes(chunk.to_vec())
            } else {
                ResponseTemplate::new(200).set_body_bytes(large_content_for_closure.clone())
            }
        })
        .mount(&mock_server)
        .await;

    let tmp = TempDir::new().unwrap();
    let blob_cache = BlobCache::new(tmp.path()).unwrap();
    let downloader = Downloader::new(blob_cache);

    let url = format!("{}/large.tar.gz", mock_server.uri());
    let result = downloader.download(&url, &actual_sha256).await;

    assert!(result.is_ok(), "Download failed: {:?}", result.err());
    let blob_path = result.unwrap();
    assert!(blob_path.exists());

    let range_count = range_requests.load(Ordering::SeqCst);
    assert!(
        range_count > 0,
        "Expected multiple Range requests, got {}",
        range_count
    );

    let downloaded_content = std::fs::read(&blob_path).unwrap();
    assert_eq!(downloaded_content.len(), large_content.len());
    assert_eq!(downloaded_content, large_content);
}

#[tokio::test]
async fn fallback_to_normal_download_when_ranges_not_supported() {
    let mock_server = MockServer::start().await;

    let large_content = vec![0xCDu8; 15 * 1024 * 1024];
    let actual_sha256 = {
        let mut hasher = Sha256::new();
        hasher.update(&large_content);
        finalize_sha256_hex(hasher)
    };

    Mock::given(method("HEAD"))
        .and(path("/large.tar.gz"))
        .respond_with(
            ResponseTemplate::new(200)
                .append_header("Content-Length", large_content.len().to_string()),
        )
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/large.tar.gz"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(large_content.clone()))
        .mount(&mock_server)
        .await;

    let tmp = TempDir::new().unwrap();
    let blob_cache = BlobCache::new(tmp.path()).unwrap();
    let downloader = Downloader::new(blob_cache);

    let url = format!("{}/large.tar.gz", mock_server.uri());
    let result = downloader.download(&url, &actual_sha256).await;

    assert!(result.is_ok());
    let blob_path = result.unwrap();
    assert!(blob_path.exists());

    let downloaded_content = std::fs::read(&blob_path).unwrap();
    assert_eq!(downloaded_content, large_content);
}

#[tokio::test]
async fn small_files_dont_use_chunked_download() {
    let mock_server = MockServer::start().await;

    let small_content = vec![0xEFu8; 1024 * 1024];
    let actual_sha256 = {
        let mut hasher = Sha256::new();
        hasher.update(&small_content);
        finalize_sha256_hex(hasher)
    };

    Mock::given(method("HEAD"))
        .and(path("/small.tar.gz"))
        .respond_with(
            ResponseTemplate::new(200)
                .append_header("Accept-Ranges", "bytes")
                .append_header("Content-Length", small_content.len().to_string()),
        )
        .mount(&mock_server)
        .await;

    let range_used = Arc::new(AtomicUsize::new(0));
    let range_used_clone = range_used.clone();
    let small_content_for_closure = small_content.clone();

    Mock::given(method("GET"))
        .and(path("/small.tar.gz"))
        .respond_with(move |req: &wiremock::Request| {
            if req.headers.get("Range").is_some() {
                range_used_clone.fetch_add(1, Ordering::SeqCst);
            }
            ResponseTemplate::new(200).set_body_bytes(small_content_for_closure.clone())
        })
        .mount(&mock_server)
        .await;

    let tmp = TempDir::new().unwrap();
    let blob_cache = BlobCache::new(tmp.path()).unwrap();
    let downloader = Downloader::new(blob_cache);

    let url = format!("{}/small.tar.gz", mock_server.uri());
    let result = downloader.download(&url, &actual_sha256).await;

    assert!(result.is_ok());
    let blob_path = result.unwrap();
    assert!(blob_path.exists());

    let range_count = range_used.load(Ordering::SeqCst);
    assert_eq!(
        range_count, 0,
        "Small files should not use chunked download"
    );

    let downloaded_content = std::fs::read(&blob_path).unwrap();
    assert_eq!(downloaded_content, small_content);
}

#[tokio::test]
async fn chunked_download_respects_concurrency_limit() {
    let mock_server = MockServer::start().await;

    let large_content = vec![0xABu8; 40 * 1024 * 1024];
    let actual_sha256 = {
        let mut hasher = Sha256::new();
        hasher.update(&large_content);
        finalize_sha256_hex(hasher)
    };

    Mock::given(method("HEAD"))
        .and(path("/large.tar.gz"))
        .respond_with(
            ResponseTemplate::new(200)
                .append_header("Accept-Ranges", "bytes")
                .append_header("Content-Length", large_content.len().to_string()),
        )
        .mount(&mock_server)
        .await;

    let concurrent_count = Arc::new(AtomicUsize::new(0));
    let max_concurrent = Arc::new(AtomicUsize::new(0));
    let concurrent_clone = concurrent_count.clone();
    let max_clone = max_concurrent.clone();
    let large_content_for_closure = large_content.clone();

    Mock::given(method("GET"))
        .and(path("/large.tar.gz"))
        .respond_with(move |req: &wiremock::Request| {
            if let Some(range_header) = req.headers.get("Range") {
                let current = concurrent_clone.fetch_add(1, Ordering::SeqCst) + 1;
                max_clone.fetch_max(current, Ordering::SeqCst);

                let range_str = range_header.to_str().unwrap();
                let range_part = range_str.strip_prefix("bytes=").unwrap();
                let (start_str, end_str) = range_part.split_once('-').unwrap();
                let start: usize = start_str.parse().unwrap();
                let end: usize = end_str.parse().unwrap();

                std::thread::sleep(Duration::from_millis(50));

                let chunk = &large_content_for_closure[start..=end];

                concurrent_clone.fetch_sub(1, Ordering::SeqCst);

                ResponseTemplate::new(206)
                    .append_header("Content-Length", chunk.len().to_string())
                    .append_header(
                        "Content-Range",
                        format!(
                            "bytes {}-{}/{}",
                            start,
                            end,
                            large_content_for_closure.len()
                        ),
                    )
                    .set_body_bytes(chunk.to_vec())
            } else {
                ResponseTemplate::new(200).set_body_bytes(large_content_for_closure.clone())
            }
        })
        .mount(&mock_server)
        .await;

    let tmp = TempDir::new().unwrap();
    let blob_cache = BlobCache::new(tmp.path()).unwrap();
    let downloader = Downloader::new(blob_cache);

    let url = format!("{}/large.tar.gz", mock_server.uri());
    let result = downloader.download(&url, &actual_sha256).await;

    assert!(result.is_ok(), "Download failed: {:?}", result.err());
    let blob_path = result.unwrap();
    assert!(blob_path.exists());

    let peak = max_concurrent.load(Ordering::SeqCst);
    assert!(
        peak <= MAX_CONCURRENT_CHUNKS,
        "Peak concurrent downloads was {peak}, expected <= {MAX_CONCURRENT_CHUNKS}"
    );

    let downloaded_content = std::fs::read(&blob_path).unwrap();
    assert_eq!(downloaded_content.len(), large_content.len());
    assert_eq!(downloaded_content, large_content);
}

#[test]
fn extract_scope_for_url_supports_core_packages() {
    let scope =
        super::extract_scope_for_url("https://ghcr.io/v2/homebrew/core/lz4/blobs/sha256:abc")
            .unwrap();
    assert_eq!(scope, "repository:homebrew/core/lz4:pull");
}

#[test]
fn extract_scope_for_url_supports_tapped_packages() {
    let scope =
        super::extract_scope_for_url("https://ghcr.io/v2/hashicorp/tap/terraform/blobs/sha256:abc")
            .unwrap();
    assert_eq!(scope, "repository:hashicorp/tap/terraform:pull");
}

#[tokio::test]
async fn download_retries_on_transient_network_failure() {
    let mock_server = MockServer::start().await;
    let content = b"retry success";
    let sha256 = {
        let mut hasher = Sha256::new();
        hasher.update(content);
        finalize_sha256_hex(hasher)
    };

    let attempt_count = Arc::new(AtomicUsize::new(0));
    let count_clone = attempt_count.clone();

    Mock::given(method("GET"))
        .and(path("/flaky.tar.gz"))
        .respond_with(move |_: &wiremock::Request| {
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
    let blob_cache = BlobCache::new(tmp.path()).unwrap();
    let downloader = Downloader::new(blob_cache);

    let url = format!("{}/flaky.tar.gz", mock_server.uri());
    let result = downloader.download(&url, &sha256).await;

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
    let sha256 = {
        let mut hasher = Sha256::new();
        hasher.update(content);
        finalize_sha256_hex(hasher)
    };

    // Setup redirect chain: /start -> /middle -> /final
    Mock::given(method("GET"))
        .and(path("/start.tar.gz"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("Location", format!("{}/middle.tar.gz", mock_server.uri())),
        )
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/middle.tar.gz"))
        .respond_with(
            ResponseTemplate::new(301)
                .insert_header("Location", format!("{}/final.tar.gz", mock_server.uri())),
        )
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/final.tar.gz"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(content.to_vec()))
        .mount(&mock_server)
        .await;

    let tmp = TempDir::new().unwrap();
    let blob_cache = BlobCache::new(tmp.path()).unwrap();
    let downloader = Downloader::new(blob_cache);

    let url = format!("{}/start.tar.gz", mock_server.uri());
    let result = downloader.download(&url, &sha256).await;

    assert!(result.is_ok(), "Should follow redirects");
    let blob_path = result.unwrap();
    assert_eq!(std::fs::read(&blob_path).unwrap(), content);
}

#[tokio::test]
async fn transform_url_to_mirror_replaces_ghcr_domain() {
    let original = "https://ghcr.io/v2/homebrew/core/wget/blobs/sha256:abc";
    let mirror = "mirror.example.com";

    let transformed = super::transform_url_to_mirror(original, mirror).unwrap();

    assert_eq!(
        transformed,
        "https://mirror.example.com/v2/homebrew/core/wget/blobs/sha256:abc"
    );
}

#[tokio::test]
async fn transform_url_to_mirror_returns_none_for_non_ghcr() {
    let original = "https://example.com/file.tar.gz";
    let mirror = "mirror.example.com";

    let result = super::transform_url_to_mirror(original, mirror);

    assert!(result.is_none());
}
