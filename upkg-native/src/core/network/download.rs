use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use futures_util::future::select_all;
use rama::{
    Service,
    error::OpaqueError,
    http::{
        Body, BodyExtractExt, HeaderValue, Request, Response, StatusCode,
        client::EasyHttpWebClient,
        header::{
            ACCEPT_RANGES, AUTHORIZATION, CONTENT_LENGTH, CONTENT_RANGE, LOCATION, WWW_AUTHENTICATE,
        },
        service::client::HttpClientExt,
    },
    net::client::pool::http::HttpPooledConnectorConfig,
    service::BoxService,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, Notify, RwLock, Semaphore, mpsc};

use crate::core::checksum::finalize_sha256_hex;
use crate::core::progress::InstallProgress;
use crate::core::storage::blob::BlobCache;
use crate::types::Error;

const RACING_CONNECTIONS: usize = 3;
const RACING_STAGGER_MS: u64 = 200;

const CHUNKED_DOWNLOAD_THRESHOLD: u64 = 10 * 1024 * 1024;

const GLOBAL_DOWNLOAD_CONCURRENCY: usize = 20;

const MAX_CONCURRENT_CHUNKS: usize = 6;

const MAX_CHUNK_RETRIES: u32 = 3;
const MAX_REDIRECTS: usize = 10;

fn calculate_chunk_size(file_size: u64) -> u64 {
    const MIN_CHUNK_SIZE: u64 = 5 * 1024 * 1024;
    const MAX_CHUNK_SIZE: u64 = 20 * 1024 * 1024;

    let target_chunks = MAX_CONCURRENT_CHUNKS as u64;
    let chunk_size = file_size / target_chunks;

    chunk_size.clamp(MIN_CHUNK_SIZE, MAX_CHUNK_SIZE)
}

struct ChunkDownloadContext {
    client: BoxService<Request, Response, OpaqueError>,
    token_cache: TokenCache,
    url: String,
    progress: Option<DownloadProgressCallback>,
    name: Option<String>,
    file_size: u64,
    total_downloaded: Arc<AtomicU64>,
}

struct ChunkedDownloadContext {
    blob_cache: BlobCache,
    client: BoxService<Request, Response, OpaqueError>,
    token_cache: TokenCache,
    url: String,
    expected_sha256: String,
    name: Option<String>,
    progress: Option<DownloadProgressCallback>,
    file_size: u64,
    global_semaphore: Arc<Semaphore>,
}

pub type DownloadProgressCallback = Arc<dyn Fn(InstallProgress) + Send + Sync>;

fn get_alternate_urls(primary_url: &str) -> Vec<String> {
    let mut alternates = Vec::new();

    if let Ok(mirrors) = std::env::var("HOMEBREW_BOTTLE_MIRRORS") {
        for mirror in mirrors.split(',') {
            let mirror = mirror.trim();
            if !mirror.is_empty()
                && let Some(alt) = transform_url_to_mirror(primary_url, mirror)
            {
                alternates.push(alt);
            }
        }
    }

    alternates
}

fn transform_url_to_mirror(url: &str, mirror_domain: &str) -> Option<String> {
    if url.contains("ghcr.io") {
        Some(url.replace("ghcr.io", mirror_domain))
    } else {
        None
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    token: String,
}

#[derive(Debug, Clone)]
pub struct DownloadResult {
    pub name: String,
    pub sha256: String,
    pub blob_path: PathBuf,
    pub index: usize,
}

struct CachedToken {
    token: String,
    expires_at: Instant,
}

type TokenCache = Arc<RwLock<HashMap<String, CachedToken>>>;

fn build_rama_client() -> BoxService<Request, Response, OpaqueError> {
    use rama::http::client::HttpClientService;

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

pub struct Downloader {
    client: BoxService<Request, Response, OpaqueError>,
    blob_cache: BlobCache,
    token_cache: TokenCache,
    global_semaphore: Option<Arc<Semaphore>>,
}

impl Downloader {
    pub fn new(blob_cache: BlobCache) -> Self {
        Self::with_semaphore(blob_cache, None)
    }

    pub fn with_semaphore(blob_cache: BlobCache, semaphore: Option<Arc<Semaphore>>) -> Self {
        Self {
            client: build_rama_client(),
            blob_cache,
            token_cache: Arc::new(RwLock::new(HashMap::new())),
            global_semaphore: semaphore,
        }
    }

    fn create_isolated_client(&self) -> BoxService<Request, Response, OpaqueError> {
        EasyHttpWebClient::connector_builder()
            .with_default_transport_connector()
            .without_tls_proxy_support()
            .with_proxy_support()
            .with_tls_support_using_rustls(None)
            .with_default_http_connector()
            .build_client()
            .boxed()
    }

    pub fn remove_blob(&self, sha256: &str) -> bool {
        self.blob_cache.remove_blob(sha256).unwrap_or(false)
    }

    pub async fn download(&self, url: &str, expected_sha256: &str) -> Result<PathBuf, Error> {
        self.download_with_progress(url, expected_sha256, None, None)
            .await
    }

    pub async fn download_with_progress(
        &self,
        url: &str,
        expected_sha256: &str,
        name: Option<String>,
        progress: Option<DownloadProgressCallback>,
    ) -> Result<PathBuf, Error> {
        if self.blob_cache.has_blob(expected_sha256) {
            if let (Some(cb), Some(n)) = (&progress, &name) {
                cb(InstallProgress::DownloadCompleted {
                    name: n.clone(),
                    total_bytes: 0,
                });
            }
            return Ok(self.blob_cache.blob_path(expected_sha256));
        }

        let alternates = get_alternate_urls(url);

        self.download_with_racing(url, &alternates, expected_sha256, name, progress)
            .await
    }

    async fn download_with_racing(
        &self,
        primary_url: &str,
        alternate_urls: &[String],
        expected_sha256: &str,
        name: Option<String>,
        progress: Option<DownloadProgressCallback>,
    ) -> Result<PathBuf, Error> {
        let (use_chunked, file_size) = {
            let cached_token =
                get_cached_token_for_url_internal(&self.token_cache, primary_url).await;

            match send_head_with_redirects(
                &self.client,
                primary_url,
                cached_token
                    .as_ref()
                    .map(|t| HeaderValue::from_str(&format!("Bearer {t}")).unwrap()),
            )
            .await
            {
                Ok(response) if response.status().is_success() => {
                    let content_length = response
                        .headers()
                        .get(CONTENT_LENGTH)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok());

                    let supports_ranges = server_supports_ranges(&response);

                    if let Some(size) = content_length {
                        (
                            supports_ranges && size >= CHUNKED_DOWNLOAD_THRESHOLD,
                            Some(size),
                        )
                    } else {
                        (false, None)
                    }
                }
                _ => (false, None),
            }
        };

        if use_chunked && let Some(size) = file_size {
            let semaphore = self
                .global_semaphore
                .clone()
                .unwrap_or_else(|| Arc::new(Semaphore::new(GLOBAL_DOWNLOAD_CONCURRENCY)));

            let mut all_urls = Vec::new();
            all_urls.push(primary_url.to_string());
            all_urls.extend(alternate_urls.iter().cloned());

            let mut last_error = None;
            for url in &all_urls {
                let ctx = ChunkedDownloadContext {
                    blob_cache: self.blob_cache.clone(),
                    client: self.client.clone(),
                    token_cache: self.token_cache.clone(),
                    url: url.clone(),
                    expected_sha256: expected_sha256.to_string(),
                    name: name.clone(),
                    progress: progress.clone(),
                    file_size: size,
                    global_semaphore: semaphore.clone(),
                };

                match download_with_chunks(&ctx).await {
                    Ok(path) => return Ok(path),
                    Err(err) => last_error = Some(err),
                }
            }

            return Err(last_error.unwrap_or_else(|| Error::NetworkFailure {
                message: "all chunked download attempts failed".to_string(),
            }));
        }

        let done = Arc::new(AtomicBool::new(false));
        let done_notify = Arc::new(Notify::new());
        let body_download_gate = Arc::new(Semaphore::new(1));

        let mut all_urls: Vec<String> = Vec::new();

        for _ in 0..RACING_CONNECTIONS {
            all_urls.push(primary_url.to_string());
        }

        all_urls.extend(alternate_urls.iter().cloned());

        let mut handles = Vec::new();
        for (idx, url) in all_urls.into_iter().enumerate() {
            let downloader_client = if idx < RACING_CONNECTIONS {
                self.create_isolated_client()
            } else {
                self.client.clone()
            };
            let blob_cache = self.blob_cache.clone();
            let token_cache = self.token_cache.clone();
            let expected_sha256 = expected_sha256.to_string();
            let name = name.clone();
            let progress = progress.clone();
            let done = done.clone();
            let done_notify = done_notify.clone();
            let body_download_gate = body_download_gate.clone();

            let delay = Duration::from_millis(idx as u64 * RACING_STAGGER_MS);

            let handle = tokio::spawn(async move {
                tokio::time::sleep(delay).await;

                if done.load(Ordering::Acquire) {
                    return Err(Error::NetworkFailure {
                        message: "cancelled: another download finished first".to_string(),
                    });
                }

                if blob_cache.has_blob(&expected_sha256) {
                    if let (Some(cb), Some(n)) = (&progress, &name) {
                        cb(InstallProgress::DownloadCompleted {
                            name: n.clone(),
                            total_bytes: 0,
                        });
                    }

                    done.store(true, Ordering::Release);
                    done_notify.notify_waiters();
                    return Ok(blob_cache.blob_path(&expected_sha256));
                }

                let response = fetch_download_response_internal(
                    downloader_client.clone(),
                    token_cache.clone(),
                    url.clone(),
                )
                .await?;

                let _permit = tokio::select! {
                    permit = body_download_gate.acquire_owned() => permit.map_err(|_| Error::NetworkFailure {
                        message: "download permit closed unexpectedly".to_string(),
                    })?,
                    _ = done_notify.notified() => {
                        return Err(Error::NetworkFailure {
                            message: "cancelled: another download finished first".to_string(),
                        });
                    }
                };

                if done.load(Ordering::Acquire) {
                    return Err(Error::NetworkFailure {
                        message: "cancelled: another download finished first".to_string(),
                    });
                }

                if blob_cache.has_blob(&expected_sha256) {
                    if let (Some(cb), Some(n)) = (&progress, &name) {
                        cb(InstallProgress::DownloadCompleted {
                            name: n.clone(),
                            total_bytes: 0,
                        });
                    }

                    done.store(true, Ordering::Release);
                    done_notify.notify_waiters();
                    return Ok(blob_cache.blob_path(&expected_sha256));
                }

                let result = download_response_internal(
                    &blob_cache,
                    response,
                    &expected_sha256,
                    name,
                    progress,
                )
                .await;

                if result.is_ok() {
                    done.store(true, Ordering::Release);
                    done_notify.notify_waiters();
                }

                result
            });

            handles.push(handle);
        }

        let mut pending = handles;
        let mut last_error = None;

        while !pending.is_empty() {
            let (result, _index, remaining) = select_all(pending).await;
            pending = remaining;

            match result {
                Ok(Ok(path)) => {
                    for handle in &pending {
                        handle.abort();
                    }
                    return Ok(path);
                }
                Ok(Err(e)) => last_error = Some(e),
                Err(e) => {
                    last_error = Some(Error::NetworkFailure {
                        message: format!("task join error: {e}"),
                    })
                }
            }
        }

        Err(last_error.unwrap_or_else(|| Error::NetworkFailure {
            message: "all download attempts failed".to_string(),
        }))
    }
}

async fn fetch_download_response_internal(
    client: BoxService<Request, Response, OpaqueError>,
    token_cache: TokenCache,
    url: String,
) -> Result<Response, Error> {
    let cached_token = get_cached_token_for_url_internal(&token_cache, &url).await;

    let response = send_get_with_redirects(
        &client,
        &url,
        cached_token
            .as_ref()
            .map(|t| HeaderValue::from_str(&format!("Bearer {t}")).unwrap()),
        None,
    )
    .await?;

    let response = if response.status() == StatusCode::UNAUTHORIZED {
        handle_auth_challenge_internal(&client, &token_cache, &url, response).await?
    } else {
        response
    };

    if !response.status().is_success() {
        return Err(Error::NetworkFailure {
            message: format!("HTTP {}", response.status()),
        });
    }

    Ok(response)
}

async fn fetch_range_response_internal(
    client: BoxService<Request, Response, OpaqueError>,
    token_cache: TokenCache,
    url: String,
    range: String,
) -> Result<Response, Error> {
    let cached_token = get_cached_token_for_url_internal(&token_cache, &url).await;

    let response = send_get_with_redirects(
        &client,
        &url,
        cached_token
            .as_ref()
            .map(|t| HeaderValue::from_str(&format!("Bearer {t}")).unwrap()),
        Some(range),
    )
    .await?;

    let response = if response.status() == StatusCode::UNAUTHORIZED {
        handle_auth_challenge_internal(&client, &token_cache, &url, response).await?
    } else {
        response
    };

    if !response.status().is_success() {
        return Err(Error::NetworkFailure {
            message: format!("HTTP {}", response.status()),
        });
    }

    Ok(response)
}

async fn get_cached_token_for_url_internal(token_cache: &TokenCache, url: &str) -> Option<String> {
    let scope = extract_scope_for_url(url)?;
    let cache = token_cache.read().await;
    let now = Instant::now();

    cache
        .get(&scope)
        .filter(|cached| cached.expires_at > now)
        .map(|cached| cached.token.clone())
}

async fn handle_auth_challenge_internal(
    client: &BoxService<Request, Response, OpaqueError>,
    token_cache: &TokenCache,
    url: &str,
    response: Response,
) -> Result<Response, Error> {
    let www_auth_header = response.headers().get(WWW_AUTHENTICATE);

    let www_auth = match www_auth_header {
        Some(value) => value.to_str().map_err(|_| Error::NetworkFailure {
            message: "WWW-Authenticate header contains invalid characters".to_string(),
        })?,
        None => {
            return Err(Error::NetworkFailure {
                message:
                    "server returned 401 without WWW-Authenticate header (may be rate limited)"
                        .to_string(),
            });
        }
    };

    let token = fetch_bearer_token_internal(client, token_cache, www_auth).await?;

    let response = send_get_with_redirects(
        client,
        url,
        Some(HeaderValue::from_str(&format!("Bearer {token}")).unwrap()),
        None,
    )
    .await?;

    if response.status() == StatusCode::UNAUTHORIZED {
        return Err(Error::NetworkFailure {
            message: "authentication failed: token was rejected by server".to_string(),
        });
    }

    Ok(response)
}

async fn fetch_bearer_token_internal(
    client: &BoxService<Request, Response, OpaqueError>,
    token_cache: &TokenCache,
    www_authenticate: &str,
) -> Result<String, Error> {
    let (realm, service, scope) = parse_www_authenticate(www_authenticate)?;

    {
        let cache = token_cache.read().await;
        if let Some(cached) = cache.get(&scope)
            && cached.expires_at > Instant::now()
        {
            return Ok(cached.token.clone());
        }
    }

    let token_url = format!("{}?service={}&scope={}", realm, service, scope);

    let response = client
        .get(&token_url)
        .send()
        .await
        .map_err(|e| Error::NetworkFailure {
            message: format!("token request failed: {e}"),
        })?;

    if !response.status().is_success() {
        return Err(Error::NetworkFailure {
            message: format!("token request returned HTTP {}", response.status()),
        });
    }

    let token_response: TokenResponse =
        response
            .try_into_json()
            .await
            .map_err(|e| Error::NetworkFailure {
                message: format!("failed to parse token response: {e}"),
            })?;

    {
        let mut cache = token_cache.write().await;
        cache.insert(
            scope,
            CachedToken {
                token: token_response.token.clone(),
                expires_at: Instant::now() + Duration::from_secs(240),
            },
        );
    }

    Ok(token_response.token)
}

fn resolve_redirect_url(current_url: &str, location: &str) -> Result<String, Error> {
    if location.starts_with("http://") || location.starts_with("https://") {
        return Ok(location.to_string());
    }

    let base = url::Url::parse(current_url).map_err(|e| Error::NetworkFailure {
        message: format!("invalid redirect base URL '{current_url}': {e}"),
    })?;
    let joined = base.join(location).map_err(|e| Error::NetworkFailure {
        message: format!("invalid redirect location '{location}': {e}"),
    })?;
    Ok(joined.to_string())
}

fn redirect_location(response: &Response) -> Result<Option<String>, Error> {
    if !response.status().is_redirection() {
        return Ok(None);
    }

    let Some(location) = response.headers().get(LOCATION) else {
        return Err(Error::NetworkFailure {
            message: format!("redirect ({}) without Location header", response.status()),
        });
    };

    let location = location.to_str().map_err(|_| Error::NetworkFailure {
        message: "redirect Location header contains invalid characters".to_string(),
    })?;
    Ok(Some(location.to_string()))
}

async fn send_get_with_redirects(
    client: &BoxService<Request, Response, OpaqueError>,
    url: &str,
    authorization: Option<HeaderValue>,
    range: Option<String>,
) -> Result<Response, Error> {
    let mut current_url = url.to_string();
    let mut redirects = 0usize;

    loop {
        let mut request = client.get(&current_url);
        if let Some(auth) = authorization.clone() {
            request = request.header(AUTHORIZATION, auth);
        }
        if let Some(ref range_header) = range {
            request = request.header("Range", range_header.as_str());
        }

        let response = request.send().await.map_err(|e| Error::NetworkFailure {
            message: e.to_string(),
        })?;

        if let Some(location) = redirect_location(&response)? {
            redirects += 1;
            if redirects > MAX_REDIRECTS {
                return Err(Error::NetworkFailure {
                    message: format!("too many redirects while fetching {url}"),
                });
            }
            current_url = resolve_redirect_url(&current_url, &location)?;
            continue;
        }

        return Ok(response);
    }
}

async fn send_head_with_redirects(
    client: &BoxService<Request, Response, OpaqueError>,
    url: &str,
    authorization: Option<HeaderValue>,
) -> Result<Response, Error> {
    let mut current_url = url.to_string();
    let mut redirects = 0usize;

    loop {
        let mut request = client.head(&current_url);
        if let Some(auth) = authorization.clone() {
            request = request.header(AUTHORIZATION, auth);
        }

        let response = request.send().await.map_err(|e| Error::NetworkFailure {
            message: e.to_string(),
        })?;

        if let Some(location) = redirect_location(&response)? {
            redirects += 1;
            if redirects > MAX_REDIRECTS {
                return Err(Error::NetworkFailure {
                    message: format!("too many redirects while fetching {url}"),
                });
            }
            current_url = resolve_redirect_url(&current_url, &location)?;
            continue;
        }

        return Ok(response);
    }
}

struct ChunkRange {
    offset: u64,
    size: u64,
}

fn server_supports_ranges(response: &Response) -> bool {
    response
        .headers()
        .get(ACCEPT_RANGES)
        .and_then(|v| v.to_str().ok())
        .map(|v| v == "bytes")
        .unwrap_or(false)
}

fn calculate_chunk_ranges(file_size: u64) -> Vec<ChunkRange> {
    let chunk_size = calculate_chunk_size(file_size);
    let mut chunks = Vec::new();
    let mut offset = 0;

    while offset < file_size {
        let remaining = file_size - offset;
        let chunk_size = remaining.min(chunk_size);
        chunks.push(ChunkRange {
            offset,
            size: chunk_size,
        });
        offset += chunk_size;
    }

    chunks
}

async fn download_chunk(ctx: &ChunkDownloadContext, chunk: &ChunkRange) -> Result<Vec<u8>, Error> {
    let range_header = format!("bytes={}-{}", chunk.offset, chunk.offset + chunk.size - 1);

    let mut last_error = None;

    for attempt in 0..=MAX_CHUNK_RETRIES {
        let cached_token = get_cached_token_for_url_internal(&ctx.token_cache, &ctx.url).await;

        match send_get_with_redirects(
            &ctx.client,
            &ctx.url,
            cached_token
                .as_ref()
                .map(|t| HeaderValue::from_str(&format!("Bearer {t}")).unwrap()),
            Some(range_header.clone()),
        )
        .await
        {
            Ok(response) => {
                if response.status() == StatusCode::UNAUTHORIZED {
                    let www_auth = match response.headers().get(WWW_AUTHENTICATE) {
                        Some(value) => value.to_str().map_err(|_| Error::NetworkFailure {
                            message: "WWW-Authenticate header contains invalid characters"
                                .to_string(),
                        })?,
                        None => {
                            return Err(Error::NetworkFailure {
                                message: "server returned 401 without WWW-Authenticate header"
                                    .to_string(),
                            });
                        }
                    };

                    match fetch_bearer_token_internal(&ctx.client, &ctx.token_cache, www_auth).await
                    {
                        Ok(_new_token) => {
                            last_error = Some(Error::NetworkFailure {
                                message: "token expired, retrying with new token".to_string(),
                            });
                            continue;
                        }
                        Err(e) => {
                            return Err(Error::NetworkFailure {
                                message: format!("failed to refresh token: {e}"),
                            });
                        }
                    }
                }

                if let Some(content_range) = response.headers().get(CONTENT_RANGE) {
                    let range_str = content_range.to_str().unwrap_or("");
                    if !range_str.contains(&format!(
                        "{}-{}",
                        chunk.offset,
                        chunk.offset + chunk.size - 1
                    )) {
                        return Err(Error::NetworkFailure {
                            message: format!(
                                "invalid content-range: expected bytes {}-{}, got: {}",
                                chunk.offset,
                                chunk.offset + chunk.size - 1,
                                range_str
                            ),
                        });
                    }
                }

                if !response.status().is_success() {
                    last_error = Some(Error::NetworkFailure {
                        message: format!("chunk download returned HTTP {}", response.status()),
                    });

                    if response.status().is_server_error() && attempt < MAX_CHUNK_RETRIES {
                        tokio::time::sleep(Duration::from_millis(100 * (1 << attempt))).await;
                        continue;
                    }
                    return Err(last_error.unwrap());
                }

                let mut chunk_data = Vec::with_capacity(chunk.size as usize);
                let mut stream = response.into_body().into_data_stream();

                while let Some(result) = stream.next().await {
                    let bytes = result.map_err(|e| Error::NetworkFailure {
                        message: format!("failed to read chunk bytes: {e}"),
                    })?;

                    chunk_data.extend_from_slice(&bytes);

                    if let (Some(cb), Some(n)) = (&ctx.progress, &ctx.name) {
                        let downloaded = ctx
                            .total_downloaded
                            .fetch_add(bytes.len() as u64, Ordering::Release);
                        cb(InstallProgress::DownloadProgress {
                            name: n.clone(),
                            downloaded: downloaded + bytes.len() as u64,
                            total_bytes: Some(ctx.file_size),
                        });
                    }
                }

                if chunk_data.len() != chunk.size as usize {
                    return Err(Error::NetworkFailure {
                        message: format!(
                            "chunk size mismatch: expected {} bytes, got {} bytes",
                            chunk.size,
                            chunk_data.len()
                        ),
                    });
                }

                return Ok(chunk_data);
            }
            Err(e) => {
                last_error = Some(Error::NetworkFailure {
                    message: format!("chunk download failed: {e}"),
                });

                if attempt < MAX_CHUNK_RETRIES {
                    tokio::time::sleep(Duration::from_millis(100 * (1 << attempt))).await;
                    continue;
                }
            }
        }
    }

    Err(last_error.unwrap_or_else(|| Error::NetworkFailure {
        message: "chunk download failed after retries".to_string(),
    }))
}

async fn download_with_chunks(ctx: &ChunkedDownloadContext) -> Result<PathBuf, Error> {
    if !validate_range_support(ctx).await? {
        let response = fetch_download_response_internal(
            ctx.client.clone(),
            ctx.token_cache.clone(),
            ctx.url.clone(),
        )
        .await?;
        return download_response_internal(
            &ctx.blob_cache,
            response,
            &ctx.expected_sha256,
            ctx.name.clone(),
            ctx.progress.clone(),
        )
        .await;
    }

    let chunks = calculate_chunk_ranges(ctx.file_size);

    if let (Some(cb), Some(n)) = (&ctx.progress, &ctx.name) {
        cb(InstallProgress::DownloadStarted {
            name: n.clone(),
            total_bytes: Some(ctx.file_size),
        });
    }

    let mut writer = ctx
        .blob_cache
        .start_write(&ctx.expected_sha256)
        .map_err(|e| Error::NetworkFailure {
            message: format!("failed to create blob writer: {e}"),
        })?;

    let expected_chunks: BTreeMap<u64, u64> = chunks.iter().map(|c| (c.offset, c.size)).collect();
    let total_chunks = chunks.len();

    let (chunk_tx, mut chunk_rx) = mpsc::unbounded_channel::<(Vec<u8>, u64)>();

    let total_downloaded = Arc::new(AtomicU64::new(0));

    let mut handles = Vec::new();
    for chunk in chunks {
        let client = ctx.client.clone();
        let token_cache = ctx.token_cache.clone();
        let url = ctx.url.to_string();
        let global_semaphore = ctx.global_semaphore.clone();
        let total_downloaded = total_downloaded.clone();
        let progress = ctx.progress.clone();
        let name = ctx.name.clone();
        let chunk_tx = chunk_tx.clone();
        let file_size = ctx.file_size;

        let handle = tokio::spawn(async move {
            let _permit = global_semaphore
                .acquire()
                .await
                .map_err(|e| Error::NetworkFailure {
                    message: format!("global semaphore error: {e}"),
                })?;

            let chunk_ctx = ChunkDownloadContext {
                client: client.clone(),
                token_cache: token_cache.clone(),
                url: url.clone(),
                progress: progress.clone(),
                name: name.clone(),
                file_size,
                total_downloaded: total_downloaded.clone(),
            };

            let chunk_data = download_chunk(&chunk_ctx, &chunk).await?;

            chunk_tx
                .send((chunk_data, chunk.offset))
                .map_err(|e| Error::NetworkFailure {
                    message: format!("failed to send chunk: {e}"),
                })?;

            Ok::<(), Error>(())
        });

        handles.push(handle);
    }

    drop(chunk_tx);

    let mut next_expected_offset: u64 = 0;
    let mut received_chunks = BTreeMap::new(); // Only buffer out-of-order chunks
    let mut chunks_written = 0u64;
    let mut hasher = Sha256::new();

    while let Some((chunk_data, offset)) = chunk_rx.recv().await {
        let expected_size = expected_chunks
            .get(&offset)
            .ok_or_else(|| Error::NetworkFailure {
                message: format!("received unexpected chunk at offset {}", offset),
            })?;

        if chunk_data.len() != *expected_size as usize {
            return Err(Error::NetworkFailure {
                message: format!(
                    "chunk size mismatch at offset {}: expected {} bytes, got {} bytes",
                    offset,
                    expected_size,
                    chunk_data.len()
                ),
            });
        }

        received_chunks.insert(offset, chunk_data);
        chunks_written += 1;

        while let Some((offset, _chunk_data)) = received_chunks.first_key_value() {
            if *offset != next_expected_offset {
                break;
            }

            let (_, chunk_data) = received_chunks.pop_first().unwrap();
            hasher.update(&chunk_data);
            writer
                .write_all(&chunk_data)
                .map_err(|e| Error::NetworkFailure {
                    message: format!(
                        "failed to write chunk at offset {}: {e}",
                        next_expected_offset
                    ),
                })?;

            next_expected_offset += chunk_data.len() as u64;
        }
    }

    for handle in handles {
        handle.await.map_err(|e| Error::NetworkFailure {
            message: format!("chunk download task failed: {e}"),
        })??;
    }

    if chunks_written as usize != total_chunks {
        return Err(Error::NetworkFailure {
            message: format!(
                "expected {} chunks, received {}",
                total_chunks, chunks_written
            ),
        });
    }

    if next_expected_offset != ctx.file_size {
        return Err(Error::NetworkFailure {
            message: format!(
                "incomplete write: expected {} bytes, wrote {} bytes",
                ctx.file_size, next_expected_offset
            ),
        });
    }

    let actual_hash = finalize_sha256_hex(hasher);

    if actual_hash != ctx.expected_sha256 {
        return Err(Error::ChecksumMismatch {
            expected: ctx.expected_sha256.to_string(),
            actual: actual_hash,
        });
    }

    writer.flush().map_err(|e| Error::NetworkFailure {
        message: format!("failed to flush download: {e}"),
    })?;

    if let (Some(cb), Some(n)) = (&ctx.progress, &ctx.name) {
        cb(InstallProgress::DownloadCompleted {
            name: n.clone(),
            total_bytes: ctx.file_size,
        });
    }

    writer.commit()
}

async fn validate_range_support(ctx: &ChunkedDownloadContext) -> Result<bool, Error> {
    let response = fetch_range_response_internal(
        ctx.client.clone(),
        ctx.token_cache.clone(),
        ctx.url.clone(),
        "bytes=0-0".to_string(),
    )
    .await?;

    if response.status() != StatusCode::PARTIAL_CONTENT {
        return Ok(false);
    }

    let content_range = response
        .headers()
        .get(CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    Ok(content_range.contains("0-0"))
}

async fn download_response_internal(
    blob_cache: &BlobCache,
    response: Response,
    expected_sha256: &str,
    name: Option<String>,
    progress: Option<DownloadProgressCallback>,
) -> Result<PathBuf, Error> {
    let total_bytes = response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok());

    if let (Some(cb), Some(n)) = (&progress, &name) {
        cb(InstallProgress::DownloadStarted {
            name: n.clone(),
            total_bytes,
        });
    }

    let mut writer =
        blob_cache
            .start_write(expected_sha256)
            .map_err(|e| Error::NetworkFailure {
                message: format!("failed to create blob writer: {e}"),
            })?;

    let mut hasher = Sha256::new();
    let mut stream = response.into_body().into_data_stream();
    let mut downloaded: u64 = 0;

    while let Some(result) = stream.next().await {
        let chunk = result.map_err(|e| Error::NetworkFailure {
            message: format!("failed to read chunk: {e}"),
        })?;

        downloaded += chunk.len() as u64;
        hasher.update(&chunk);
        writer
            .write_all(&chunk)
            .map_err(|e| Error::NetworkFailure {
                message: format!("failed to write chunk: {e}"),
            })?;

        if let (Some(cb), Some(n)) = (&progress, &name) {
            cb(InstallProgress::DownloadProgress {
                name: n.clone(),
                downloaded,
                total_bytes,
            });
        }
    }

    let actual_hash = finalize_sha256_hex(hasher);

    if actual_hash != expected_sha256 {
        return Err(Error::ChecksumMismatch {
            expected: expected_sha256.to_string(),
            actual: actual_hash,
        });
    }

    writer.flush().map_err(|e| Error::NetworkFailure {
        message: format!("failed to flush download: {e}"),
    })?;

    if let (Some(cb), Some(n)) = (&progress, &name) {
        cb(InstallProgress::DownloadCompleted {
            name: n.clone(),
            total_bytes: downloaded,
        });
    }

    writer.commit()
}

fn extract_scope_for_url(url: &str) -> Option<String> {
    let marker = "ghcr.io/v2/";
    let start = url.find(marker)? + marker.len();
    let remainder = &url[start..];
    let mut parts = remainder.split('/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    let formula = parts.next()?;
    if owner.is_empty() || repo.is_empty() || formula.is_empty() {
        return None;
    }
    Some(format!("repository:{owner}/{repo}/{formula}:pull"))
}

fn parse_www_authenticate(header: &str) -> Result<(String, String, String), Error> {
    let header = header
        .strip_prefix("Bearer ")
        .ok_or_else(|| Error::NetworkFailure {
            message: "unsupported auth scheme".to_string(),
        })?;

    let mut realm = None;
    let mut service = None;
    let mut scope = None;

    for part in header.split(',') {
        let part = part.trim();
        if let Some((key, value)) = part.split_once('=') {
            let value = value.trim_matches('"');
            match key {
                "realm" => realm = Some(value.to_string()),
                "service" => service = Some(value.to_string()),
                "scope" => scope = Some(value.to_string()),
                _ => {}
            }
        }
    }

    let realm = realm.ok_or_else(|| Error::NetworkFailure {
        message: "missing realm in WWW-Authenticate".to_string(),
    })?;
    let service = service.ok_or_else(|| Error::NetworkFailure {
        message: "missing service in WWW-Authenticate".to_string(),
    })?;
    let scope = scope.ok_or_else(|| Error::NetworkFailure {
        message: "missing scope in WWW-Authenticate".to_string(),
    })?;

    Ok((realm, service, scope))
}

pub struct DownloadRequest {
    pub url: String,
    pub sha256: String,
    pub name: String,
}

type InflightMap = HashMap<String, Arc<tokio::sync::broadcast::Sender<Result<PathBuf, String>>>>;

pub struct ParallelDownloader {
    downloader: Arc<Downloader>,
    semaphore: Arc<Semaphore>,
    inflight: Arc<Mutex<InflightMap>>,
}

impl ParallelDownloader {
    pub fn new(blob_cache: BlobCache) -> Self {
        let semaphore = Arc::new(Semaphore::new(GLOBAL_DOWNLOAD_CONCURRENCY));
        Self {
            downloader: Arc::new(Downloader::with_semaphore(
                blob_cache,
                Some(semaphore.clone()),
            )),
            semaphore,
            inflight: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn with_concurrency(blob_cache: BlobCache, concurrency: usize) -> Self {
        let semaphore = Arc::new(Semaphore::new(concurrency));
        Self {
            downloader: Arc::new(Downloader::with_semaphore(
                blob_cache,
                Some(semaphore.clone()),
            )),
            semaphore,
            inflight: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn remove_blob(&self, sha256: &str) -> bool {
        self.downloader.remove_blob(sha256)
    }

    pub async fn download_single(
        &self,
        request: DownloadRequest,
        progress: Option<DownloadProgressCallback>,
    ) -> Result<PathBuf, Error> {
        Self::download_with_dedup(
            self.downloader.clone(),
            self.semaphore.clone(),
            self.inflight.clone(),
            request,
            progress,
        )
        .await
    }

    pub async fn download_all(
        &self,
        requests: Vec<DownloadRequest>,
    ) -> Result<Vec<PathBuf>, Error> {
        self.download_all_with_progress(requests, None).await
    }

    pub async fn download_all_with_progress(
        &self,
        requests: Vec<DownloadRequest>,
        progress: Option<DownloadProgressCallback>,
    ) -> Result<Vec<PathBuf>, Error> {
        let handles: Vec<_> = requests
            .into_iter()
            .map(|req| {
                let downloader = self.downloader.clone();
                let semaphore = self.semaphore.clone();
                let inflight = self.inflight.clone();
                let progress = progress.clone();

                tokio::spawn(async move {
                    Self::download_with_dedup(downloader, semaphore, inflight, req, progress).await
                })
            })
            .collect();

        let mut results = Vec::with_capacity(handles.len());
        for handle in handles {
            let result = handle.await.map_err(|e| Error::NetworkFailure {
                message: format!("task join error: {e}"),
            })??;
            results.push(result);
        }

        Ok(results)
    }

    pub fn download_streaming(
        &self,
        requests: Vec<DownloadRequest>,
        progress: Option<DownloadProgressCallback>,
    ) -> mpsc::Receiver<Result<DownloadResult, Error>> {
        let (tx, rx) = mpsc::channel(requests.len().max(1));

        for (index, req) in requests.into_iter().enumerate() {
            let downloader = self.downloader.clone();
            let semaphore = self.semaphore.clone();
            let inflight = self.inflight.clone();
            let progress = progress.clone();
            let tx = tx.clone();
            let name = req.name.clone();
            let sha256 = req.sha256.clone();

            tokio::spawn(async move {
                let result =
                    Self::download_with_dedup(downloader, semaphore, inflight, req, progress).await;
                let _ = tx
                    .send(result.map(|blob_path| DownloadResult {
                        name,
                        sha256,
                        blob_path,
                        index,
                    }))
                    .await;
            });
        }

        rx
    }

    async fn download_with_dedup(
        downloader: Arc<Downloader>,
        semaphore: Arc<Semaphore>,
        inflight: Arc<Mutex<InflightMap>>,
        req: DownloadRequest,
        progress: Option<DownloadProgressCallback>,
    ) -> Result<PathBuf, Error> {
        let mut receiver = {
            let mut map = inflight.lock().await;

            if let Some(sender) = map.get(&req.sha256) {
                Some(sender.subscribe())
            } else {
                let (tx, _) = tokio::sync::broadcast::channel(1);
                map.insert(req.sha256.clone(), Arc::new(tx));
                None
            }
        };

        if let Some(ref mut rx) = receiver {
            let result = rx.recv().await.map_err(|e| Error::NetworkFailure {
                message: format!("broadcast recv error: {e}"),
            })?;

            return result.map_err(|msg| Error::NetworkFailure { message: msg });
        }

        let _permit = semaphore
            .acquire()
            .await
            .map_err(|e| Error::NetworkFailure {
                message: format!("semaphore error: {e}"),
            })?;

        let result = downloader
            .download_with_progress(&req.url, &req.sha256, Some(req.name), progress)
            .await;

        {
            let mut map = inflight.lock().await;
            if let Some(sender) = map.remove(&req.sha256) {
                let broadcast_result = match &result {
                    Ok(path) => Ok(path.clone()),
                    Err(e) => Err(e.to_string()),
                };
                let _ = sender.send(broadcast_result);
            }
        }

        result
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use tempfile::TempDir;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn build_rama_client_does_not_panic() {
        let _ = build_rama_client();
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
        assert!(matches!(err, Error::ChecksumMismatch { .. }));

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
                        .append_header("Content-Range", format!("bytes {}-{}/{}", start, end, large_content_for_closure.len()))
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
        let scope = super::extract_scope_for_url(
            "https://ghcr.io/v2/hashicorp/tap/terraform/blobs/sha256:abc",
        )
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
}
