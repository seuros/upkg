use crate::checksum::verify_sha256_bytes;
use crate::core::network::cache::{ApiCache, CacheEntry};
use crate::core::network::tap_formula::{
    TapFormulaRef, parse_tap_formula_ref, parse_tap_formula_ruby,
};
use crate::http_client::{self, RamaClient};
use crate::package_ref::cask_name;
use crate::types::{Error, Formula};
use futures_util::stream::{self, StreamExt};
use rama::http::{BodyExtractExt, Response, StatusCode, service::client::HttpClientExt};

const HOMEBREW_CORE_RAW_BASE: &str =
    "https://raw.githubusercontent.com/Homebrew/homebrew-core/main";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RubySourceLocator<'a> {
    CoreRelativePath(&'a str),
    AbsoluteUrl(&'a str),
    TapEncodedUrl(&'a str),
    LocalPath(&'a str),
}

impl<'a> RubySourceLocator<'a> {
    const TAP_URL_PREFIX: &'static str = "tap-rb-url:";

    fn parse(input: &'a str) -> Self {
        if let Some(encoded_url) = input.strip_prefix(Self::TAP_URL_PREFIX) {
            return Self::TapEncodedUrl(encoded_url);
        }

        if input.starts_with("https://") || input.starts_with("http://") {
            return Self::AbsoluteUrl(input);
        }

        if input.starts_with('/') || input.starts_with("file://") {
            return Self::LocalPath(input.strip_prefix("file://").unwrap_or(input));
        }

        Self::CoreRelativePath(input)
    }

    fn source_id(self, original: &'a str) -> &'a str {
        match self {
            Self::CoreRelativePath(_) => original,
            Self::AbsoluteUrl(url) => url,
            Self::TapEncodedUrl(url) => url,
            Self::LocalPath(path) => path,
        }
    }

    fn to_url(self) -> String {
        match self {
            Self::CoreRelativePath(path) => format!("{HOMEBREW_CORE_RAW_BASE}/{path}"),
            Self::AbsoluteUrl(url) | Self::TapEncodedUrl(url) => url.to_string(),
            Self::LocalPath(path) => path.to_string(),
        }
    }

    fn encode_tap_url(url: &str) -> String {
        format!("{}{}", Self::TAP_URL_PREFIX, url)
    }
}

pub struct ApiClient {
    base_url: String,
    cask_base_url: String,
    tap_raw_base_url: String,
    tap_roots: Vec<std::path::PathBuf>,
    client: RamaClient,
    cache: Option<ApiCache>,
}

const INDEX_TTL_SECONDS: u64 = 12 * 3600;

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct IndexMeta {
    etag: Option<String>,
    last_modified: Option<String>,
    fetched_at: u64,
}

impl ApiClient {
    pub fn new() -> Self {
        Self::with_base_url("https://formulae.brew.sh/api/formula".to_string())
    }

    pub fn with_base_url(base_url: String) -> Self {
        Self {
            base_url,
            cask_base_url: "https://formulae.brew.sh/api/cask".to_string(),
            tap_raw_base_url: "https://raw.githubusercontent.com".to_string(),
            tap_roots: default_tap_roots(),
            client: http_client::build_rama_client(),
            cache: None,
        }
    }

    #[cfg(test)]
    pub fn with_tap_raw_base_url(mut self, tap_raw_base_url: String) -> Self {
        self.tap_raw_base_url = tap_raw_base_url;
        self
    }

    #[cfg(test)]
    pub fn with_cask_base_url(mut self, cask_base_url: String) -> Self {
        self.cask_base_url = cask_base_url;
        self
    }

    #[cfg(test)]
    pub fn with_tap_roots(mut self, tap_roots: Vec<std::path::PathBuf>) -> Self {
        self.tap_roots = tap_roots;
        self
    }

    #[cfg(test)]
    pub fn with_cache(mut self, cache: ApiCache) -> Self {
        self.cache = Some(cache);
        self
    }

    pub async fn fetch_formula_rb(
        &self,
        ruby_source_path: &str,
        cache_dir: &std::path::Path,
        expected_sha256: Option<&str>,
    ) -> Result<std::path::PathBuf, Error> {
        let locator = RubySourceLocator::parse(ruby_source_path);
        if let RubySourceLocator::LocalPath(path) = locator {
            return Ok(std::path::PathBuf::from(path));
        }

        let source_id = locator.source_id(ruby_source_path);
        let url = locator.to_url();

        self.fetch_formula_rb_from_url(source_id, &url, cache_dir, expected_sha256)
            .await
    }

    async fn fetch_formula_rb_from_url(
        &self,
        ruby_source_path: &str,
        url: &str,
        cache_dir: &std::path::Path,
        expected_sha256: Option<&str>,
    ) -> Result<std::path::PathBuf, Error> {
        let cache_key = format!("rb:{url}");
        if let Some(entry) = self.cache.as_ref().and_then(|c| c.get(&cache_key)) {
            verify_sha256_bytes(entry.body.as_bytes(), expected_sha256)
                .map_err(|e| Self::map_formula_rb_checksum_error(e, ruby_source_path, "cache"))?;

            return write_rb_file(cache_dir, ruby_source_path, &entry.body);
        }

        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| Error::NetworkFailure {
                message: format!("failed to fetch formula rb: {e}"),
            })?;

        if !response.status().is_success() {
            return Err(Error::NetworkFailure {
                message: format!("formula rb fetch returned HTTP {}", response.status()),
            });
        }

        let body = response
            .try_into_string()
            .await
            .map_err(|e| Error::NetworkFailure {
                message: format!("failed to read formula rb response: {e}"),
            })?;

        verify_sha256_bytes(body.as_bytes(), expected_sha256)
            .map_err(|e| Self::map_formula_rb_checksum_error(e, ruby_source_path, "network"))?;

        if let Some(ref cache) = self.cache {
            let entry = CacheEntry {
                etag: None,
                last_modified: None,
                body: body.clone(),
            };
            let _ = cache.put(&cache_key, &entry);
        }

        write_rb_file(cache_dir, ruby_source_path, &body)
    }

    fn map_formula_rb_checksum_error(err: Error, ruby_source_path: &str, source: &str) -> Error {
        match err {
            Error::ChecksumMismatch { .. } => err,
            Error::InvalidArgument { message } => Error::InvalidArgument {
                message: format!(
                    "invalid ruby_source_checksum for '{ruby_source_path}' (source: {source}): {message}"
                ),
            },
            other => other,
        }
    }

    pub async fn get_formula(&self, name: &str) -> Result<Formula, Error> {
        if let Some(spec) = parse_tap_formula_ref(name) {
            return self.get_tap_formula(&spec).await;
        }

        let url = format!("{}/{}.json", self.base_url, name);

        let cached_entry = self.cache.as_ref().and_then(|c| c.get(&url));

        let mut request = self.client.get(&url);

        if let Some(ref entry) = cached_entry {
            if let Some(ref etag) = entry.etag {
                request = request.header("If-None-Match", etag.as_str());
            }
            if let Some(ref last_modified) = entry.last_modified {
                request = request.header("If-Modified-Since", last_modified.as_str());
            }
        }

        let response = match request.send().await {
            Ok(response) => response,
            Err(e) => {
                if let Some(formula) = self.get_local_tap_formula(name)? {
                    return Ok(formula);
                }

                return Err(Error::NetworkFailure {
                    message: e.to_string(),
                });
            }
        };

        if response.status() == StatusCode::NOT_MODIFIED
            && let Some(entry) = cached_entry
        {
            let formula: Formula =
                serde_json::from_str(&entry.body).map_err(|e| Error::NetworkFailure {
                    message: format!("failed to parse cached formula JSON: {e}"),
                })?;
            return Ok(formula);
        }

        if response.status() == StatusCode::NOT_FOUND {
            if let Some(formula) = self.get_local_tap_formula(name)? {
                return Ok(formula);
            }

            return Err(Error::MissingFormula {
                name: name.to_string(),
            });
        }

        let response = super::ensure_success(response)?;

        let etag = header_string(&response, "etag");

        let last_modified = header_string(&response, "last-modified");

        let body = response
            .try_into_string()
            .await
            .map_err(|e| Error::NetworkFailure {
                message: format!("failed to read response body: {e}"),
            })?;

        if let Some(ref cache) = self.cache {
            let entry = CacheEntry {
                etag,
                last_modified,
                body: body.clone(),
            };
            let _ = cache.put(&url, &entry);
        }

        let formula: Formula = serde_json::from_str(&body).map_err(|e| Error::NetworkFailure {
            message: format!("failed to parse formula JSON: {e}"),
        })?;

        Ok(formula)
    }

    pub async fn fetch_formula_index(
        &self,
        cache_dir: &std::path::Path,
        refresh: bool,
    ) -> Result<String, Error> {
        let url = format!("{}.json", self.base_url.trim_end_matches('/'));
        self.fetch_index(&url, cache_dir, "formula", refresh).await
    }

    pub async fn fetch_cask_index(
        &self,
        cache_dir: &std::path::Path,
        refresh: bool,
    ) -> Result<String, Error> {
        let url = format!("{}.json", self.cask_base_url.trim_end_matches('/'));
        self.fetch_index(&url, cache_dir, "cask", refresh).await
    }

    async fn fetch_index(
        &self,
        url: &str,
        cache_dir: &std::path::Path,
        slug: &str,
        refresh: bool,
    ) -> Result<String, Error> {
        std::fs::create_dir_all(cache_dir).map_err(|e| Error::FileError {
            message: format!("failed to create search cache dir: {e}"),
        })?;

        let body_path = cache_dir.join(format!("{slug}.json"));
        let meta_path = cache_dir.join(format!("{slug}.meta.json"));

        let meta = load_index_meta(&meta_path);
        let cached_body = if body_path.exists() {
            std::fs::read_to_string(&body_path).ok()
        } else {
            None
        };

        if !refresh
            && let (Some(meta), Some(body)) = (meta.as_ref(), cached_body.as_ref())
            && index_is_fresh(meta.fetched_at)
        {
            return Ok(body.clone());
        }

        let mut request = self.client.get(url);
        if let Some(meta) = meta.as_ref() {
            if let Some(etag) = meta.etag.as_deref() {
                request = request.header("If-None-Match", etag);
            }
            if let Some(last_modified) = meta.last_modified.as_deref() {
                request = request.header("If-Modified-Since", last_modified);
            }
        }

        let response = match request.send().await {
            Ok(response) => response,
            Err(e) => {
                if let Some(body) = cached_body {
                    eprintln!("warning: using stale cached {slug} index (network: {})", e);
                    return Ok(body);
                }
                return Err(Error::NetworkFailure {
                    message: format!("failed to fetch {slug} index: {e}"),
                });
            }
        };

        if response.status() == StatusCode::NOT_MODIFIED {
            if let Some(body) = cached_body {
                let mut updated = meta.unwrap_or_default();
                updated.fetched_at = now_secs();
                save_index_meta(&meta_path, &updated);
                return Ok(body);
            }
            return Err(Error::NetworkFailure {
                message: format!("{slug} index returned 304 but no cached body exists"),
            });
        }

        if !response.status().is_success() {
            if let Some(body) = cached_body {
                eprintln!(
                    "warning: using stale cached {slug} index (server: HTTP {})",
                    response.status()
                );
                return Ok(body);
            }
            return Err(Error::NetworkFailure {
                message: format!("{slug} index returned HTTP {}", response.status()),
            });
        }

        let etag = header_string(&response, "etag");
        let last_modified = header_string(&response, "last-modified");

        let body = response
            .try_into_string()
            .await
            .map_err(|e| Error::NetworkFailure {
                message: format!("failed to read {slug} index body: {e}"),
            })?;

        let pid = std::process::id();
        let tid = std::thread::current().id();
        let tmp_path = cache_dir.join(format!("{slug}.json.{pid}.{tid:?}.part"));
        std::fs::write(&tmp_path, &body).map_err(|e| Error::FileError {
            message: format!("failed to write {slug} index part: {e}"),
        })?;
        if let Err(e) = std::fs::rename(&tmp_path, &body_path) {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(Error::FileError {
                message: format!("failed to commit {slug} index: {e}"),
            });
        }

        save_index_meta(
            &meta_path,
            &IndexMeta {
                etag,
                last_modified,
                fetched_at: now_secs(),
            },
        );

        Ok(body)
    }

    pub async fn get_cask(&self, token: &str) -> Result<serde_json::Value, Error> {
        if let Some(cask) = self.get_cask_exact(token).await? {
            return Ok(cask);
        }

        if let Some(current_token) = self.resolve_cask_old_token(token).await?
            && let Some(cask) = self.get_cask_exact(&current_token).await?
        {
            return Ok(cask);
        }

        Err(Error::MissingFormula {
            name: cask_name(token),
        })
    }

    async fn get(&self, url: &str) -> Result<Response, Error> {
        self.client
            .get(url)
            .send()
            .await
            .map_err(|e| Error::NetworkFailure {
                message: e.to_string(),
            })
    }

    async fn get_cask_exact(&self, token: &str) -> Result<Option<serde_json::Value>, Error> {
        let url = format!("{}/{}.json", self.cask_base_url, token);
        let response = self.get(&url).await?;

        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }

        let response = super::ensure_success(response)?;

        response
            .try_into_json::<serde_json::Value>()
            .await
            .map(Some)
            .map_err(|e| Error::NetworkFailure {
                message: format!("failed to parse cask JSON: {e}"),
            })
    }

    async fn resolve_cask_old_token(&self, token: &str) -> Result<Option<String>, Error> {
        let url = format!("{}.json", self.cask_base_url.trim_end_matches('/'));
        let response = self.get(&url).await?;

        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(Error::NetworkFailure {
                message: format!("cask index returned HTTP {}", response.status()),
            });
        }

        let casks = response
            .try_into_json::<Vec<serde_json::Value>>()
            .await
            .map_err(|e| Error::NetworkFailure {
                message: format!("failed to parse cask index JSON: {e}"),
            })?;

        Ok(casks.into_iter().find_map(|cask| {
            let is_old_token = cask
                .get("old_tokens")
                .and_then(serde_json::Value::as_array)
                .map(|tokens| tokens.iter().any(|old| old.as_str() == Some(token)))
                .unwrap_or(false);
            is_old_token
                .then(|| cask.get("token").and_then(serde_json::Value::as_str))
                .flatten()
                .map(ToString::to_string)
        }))
    }

    async fn get_tap_formula(&self, spec: &TapFormulaRef) -> Result<Formula, Error> {
        let candidate_repos = if spec.repo.starts_with("homebrew-") {
            vec![
                spec.repo.clone(),
                spec.repo.trim_start_matches("homebrew-").to_string(),
            ]
        } else {
            vec![format!("homebrew-{}", spec.repo), spec.repo.clone()]
        };
        let candidate_paths = tap_formula_candidate_paths(&spec.formula);
        let branches = ["main", "master"];

        let mut last_status: Option<StatusCode> = None;
        let mut last_network_error: Option<Error> = None;
        let mut saw_non_404_status = false;

        for repo in candidate_repos {
            for branch in branches {
                let base_prefix = format!(
                    "{}/{}/{}/{}/",
                    self.tap_raw_base_url.trim_end_matches('/'),
                    spec.owner,
                    repo,
                    branch,
                );
                let client = self.client.clone();
                let mut responses = stream::iter(candidate_paths.iter().map(|candidate_path| {
                    let client = client.clone();
                    let url = format!("{base_prefix}{candidate_path}");
                    async move { (url.clone(), client.get(&url).send().await) }
                }))
                .buffered(2);

                while let Some((url, response)) = responses.next().await {
                    match response {
                        Ok(response) => {
                            let status = response.status();
                            if status.is_success() {
                                let body = response.try_into_string().await.map_err(|e| {
                                    Error::NetworkFailure {
                                        message: format!("failed to read tap formula body: {e}"),
                                    }
                                })?;
                                let mut formula = parse_tap_formula_ruby(spec, &body)?;
                                formula.ruby_source_path =
                                    Some(RubySourceLocator::encode_tap_url(&url));
                                return Ok(formula);
                            }

                            if status != StatusCode::NOT_FOUND {
                                saw_non_404_status = true;
                            }
                            last_status = Some(status);
                        }
                        Err(e) => {
                            last_network_error = Some(Error::NetworkFailure {
                                message: e.to_string(),
                            });
                        }
                    }
                }
            }
        }

        if !saw_non_404_status
            && last_network_error.is_none()
            && last_status == Some(StatusCode::NOT_FOUND)
        {
            return Err(Error::MissingFormula {
                name: format!("{}/{}/{}", spec.owner, spec.repo, spec.formula),
            });
        }

        if let Some(err) = last_network_error {
            return Err(err);
        }

        Err(Error::NetworkFailure {
            message: format!(
                "failed to fetch tap formula '{}/{}/{}' (last status: {})",
                spec.owner,
                spec.repo,
                spec.formula,
                last_status
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "unknown".to_string())
            ),
        })
    }

    fn get_local_tap_formula(&self, name: &str) -> Result<Option<Formula>, Error> {
        if name.contains('/') {
            return Ok(None);
        }

        for root in &self.tap_roots {
            if !root.is_dir() {
                continue;
            }

            for owner_dir in sorted_dirs(root)? {
                let Some(owner) = file_name_string(&owner_dir) else {
                    continue;
                };

                for repo_dir in sorted_dirs(&owner_dir)? {
                    let Some(repo_dir_name) = file_name_string(&repo_dir) else {
                        continue;
                    };
                    let repo = repo_dir_name
                        .strip_prefix("homebrew-")
                        .unwrap_or(&repo_dir_name)
                        .to_string();

                    for candidate_path in tap_formula_candidate_paths(name) {
                        let formula_path = repo_dir.join(&candidate_path);
                        if !formula_path.is_file() {
                            continue;
                        }

                        let source = std::fs::read_to_string(&formula_path).map_err(|e| {
                            Error::FileError {
                                message: format!(
                                    "failed to read local tap formula '{}': {e}",
                                    formula_path.display()
                                ),
                            }
                        })?;
                        let spec = TapFormulaRef {
                            owner: owner.clone(),
                            repo: repo.clone(),
                            formula: name.to_string(),
                        };
                        let mut formula = parse_tap_formula_ruby(&spec, &source)?;
                        formula.ruby_source_path = Some(formula_path.display().to_string());
                        return Ok(Some(formula));
                    }
                }
            }
        }

        Ok(None)
    }
}

fn header_string(response: &Response, name: &str) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(ToString::to_string)
}

fn write_rb_file(
    cache_dir: &std::path::Path,
    ruby_source_path: &str,
    body: &str,
) -> Result<std::path::PathBuf, Error> {
    let dest = cache_dir.join(ruby_source_path.replace('/', "_"));
    std::fs::create_dir_all(cache_dir).map_err(|e| Error::FileError {
        message: format!("failed to create rb cache dir: {e}"),
    })?;
    std::fs::write(&dest, body.as_bytes()).map_err(|e| Error::FileError {
        message: format!("failed to write rb file: {e}"),
    })?;
    Ok(dest)
}

fn now_secs() -> u64 {
    crate::clock::unix_secs() as u64
}

fn index_is_fresh(fetched_at: u64) -> bool {
    let now = now_secs();
    now >= fetched_at && now - fetched_at < INDEX_TTL_SECONDS
}

fn load_index_meta(meta_path: &std::path::Path) -> Option<IndexMeta> {
    let body = std::fs::read_to_string(meta_path).ok()?;
    serde_json::from_str(&body).ok()
}

fn save_index_meta(meta_path: &std::path::Path, meta: &IndexMeta) {
    if let Ok(body) = serde_json::to_string(meta) {
        let _ = std::fs::write(meta_path, body);
    }
}

fn tap_formula_candidate_paths(formula: &str) -> Vec<String> {
    let first_char = formula.chars().next().unwrap_or('x');
    vec![
        format!("Formula/{formula}.rb"),
        format!("Formula/{first_char}/{formula}.rb"),
        format!("HomebrewFormula/{formula}.rb"),
        format!("HomebrewFormula/{first_char}/{formula}.rb"),
        format!("{formula}.rb"),
    ]
}

fn sorted_dirs(path: &std::path::Path) -> Result<Vec<std::path::PathBuf>, Error> {
    let mut dirs = Vec::new();
    for entry in std::fs::read_dir(path).map_err(|e| Error::FileError {
        message: format!("failed to read tap directory '{}': {e}", path.display()),
    })? {
        let entry = entry.map_err(|e| Error::FileError {
            message: format!("failed to read tap directory entry: {e}"),
        })?;
        let path = entry.path();
        if path.is_dir() {
            dirs.push(path);
        }
    }
    dirs.sort();
    Ok(dirs)
}

fn file_name_string(path: &std::path::Path) -> Option<String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(ToString::to_string)
}

fn default_tap_roots() -> Vec<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    {
        vec![
            std::path::PathBuf::from("/opt/homebrew/Library/Taps"),
            std::path::PathBuf::from("/usr/local/Homebrew/Library/Taps"),
            std::path::PathBuf::from("/usr/local/Library/Taps"),
        ]
    }

    #[cfg(not(target_os = "macos"))]
    {
        Vec::new()
    }
}

impl Default for ApiClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(all(test, target_os = "macos"))]
#[path = "api/tests.rs"]
mod tests;
