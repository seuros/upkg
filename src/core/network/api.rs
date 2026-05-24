use crate::checksum::verify_sha256_bytes;
use crate::core::network::cache::{ApiCache, CacheEntry};
use crate::core::network::tap_formula::{
    TapFormulaRef, parse_tap_formula_ref, parse_tap_formula_ruby,
};
use crate::http_client::{self, RamaClient};
use crate::package_ref::cask_name;
use crate::types::{Error, Formula};
use futures_util::stream::{self, StreamExt};
use rama::http::{BodyExtractExt, StatusCode, service::client::HttpClientExt};

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

            let dest = cache_dir.join(ruby_source_path.replace('/', "_"));
            std::fs::create_dir_all(cache_dir).map_err(|e| Error::FileError {
                message: format!("failed to create rb cache dir: {e}"),
            })?;
            std::fs::write(&dest, entry.body.as_bytes()).map_err(|e| Error::FileError {
                message: format!("failed to write cached rb file: {e}"),
            })?;
            return Ok(dest);
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

        let dest = cache_dir.join(ruby_source_path.replace('/', "_"));
        std::fs::create_dir_all(cache_dir).map_err(|e| Error::FileError {
            message: format!("failed to create rb cache dir: {e}"),
        })?;
        std::fs::write(&dest, body.as_bytes()).map_err(|e| Error::FileError {
            message: format!("failed to write rb file: {e}"),
        })?;

        Ok(dest)
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

        if !response.status().is_success() {
            return Err(Error::NetworkFailure {
                message: format!("HTTP {}", response.status()),
            });
        }

        let etag = response
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let last_modified = response
            .headers()
            .get("last-modified")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

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

    pub async fn get_cask(&self, token: &str) -> Result<serde_json::Value, Error> {
        let url = format!("{}/{}.json", self.cask_base_url, token);
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| Error::NetworkFailure {
                message: e.to_string(),
            })?;

        if response.status() == StatusCode::NOT_FOUND {
            return Err(Error::MissingFormula {
                name: cask_name(token),
            });
        }

        if !response.status().is_success() {
            return Err(Error::NetworkFailure {
                message: format!("HTTP {}", response.status()),
            });
        }

        response
            .try_into_json::<serde_json::Value>()
            .await
            .map_err(|e| Error::NetworkFailure {
                message: format!("failed to parse cask JSON: {e}"),
            })
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
mod tests {
    use super::*;
    use tempfile::tempdir;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn ruby_source_locator_parses_all_supported_kinds() {
        assert_eq!(
            RubySourceLocator::parse("Formula/f/foo.rb"),
            RubySourceLocator::CoreRelativePath("Formula/f/foo.rb")
        );
        assert_eq!(
            RubySourceLocator::parse("https://example.com/foo.rb"),
            RubySourceLocator::AbsoluteUrl("https://example.com/foo.rb")
        );
        assert_eq!(
            RubySourceLocator::parse("/opt/homebrew/Library/Taps/me/homebrew-tools/Formula/foo.rb"),
            RubySourceLocator::LocalPath(
                "/opt/homebrew/Library/Taps/me/homebrew-tools/Formula/foo.rb"
            )
        );
        assert_eq!(
            RubySourceLocator::parse("file:///tmp/foo.rb"),
            RubySourceLocator::LocalPath("/tmp/foo.rb")
        );
        let encoded = format!(
            "{}{}",
            RubySourceLocator::TAP_URL_PREFIX,
            "https://example.com/tap/foo.rb"
        );
        assert_eq!(
            RubySourceLocator::parse(&encoded),
            RubySourceLocator::TapEncodedUrl("https://example.com/tap/foo.rb")
        );
    }

    #[test]
    fn ruby_source_locator_resolves_urls_exhaustively() {
        assert_eq!(
            RubySourceLocator::CoreRelativePath("Formula/f/foo.rb").to_url(),
            "https://raw.githubusercontent.com/Homebrew/homebrew-core/main/Formula/f/foo.rb"
        );
        assert_eq!(
            RubySourceLocator::AbsoluteUrl("https://example.com/foo.rb").to_url(),
            "https://example.com/foo.rb"
        );
        assert_eq!(
            RubySourceLocator::TapEncodedUrl(
                "https://raw.githubusercontent.com/org/tap/main/foo.rb"
            )
            .to_url(),
            "https://raw.githubusercontent.com/org/tap/main/foo.rb"
        );
        assert_eq!(
            RubySourceLocator::LocalPath("/tmp/foo.rb").to_url(),
            "/tmp/foo.rb"
        );
    }

    #[tokio::test]
    async fn fetches_formula_from_mock_server() {
        let mock_server = MockServer::start().await;

        let fixture = include_str!("../../fixtures/formula_foo.json");

        Mock::given(method("GET"))
            .and(path("/foo.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
            .mount(&mock_server)
            .await;

        let client = ApiClient::with_base_url(mock_server.uri());
        let formula = client.get_formula("foo").await.unwrap();

        assert_eq!(formula.name, "foo");
        assert_eq!(formula.versions.stable, "1.2.3");
    }

    #[tokio::test]
    async fn returns_missing_formula_on_404() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/nonexistent.json"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        let client = ApiClient::with_base_url(mock_server.uri());
        let err = client.get_formula("nonexistent").await.unwrap_err();

        assert!(matches!(
            err,
            Error::MissingFormula { name } if name == "nonexistent"
        ));
    }

    #[tokio::test]
    async fn resolves_short_name_from_local_tap_after_core_404() {
        let mock_server = MockServer::start().await;
        let tap_root = tempdir().unwrap();
        let formula_dir = tap_root
            .path()
            .join("eugene1g")
            .join("homebrew-safehouse")
            .join("Formula");
        std::fs::create_dir_all(&formula_dir).unwrap();
        let formula_path = formula_dir.join("agent-safehouse.rb");
        std::fs::write(
            &formula_path,
            r#"
class AgentSafehouse < Formula
  desc "macOS sandbox wrapper for coding agents"
  homepage "https://github.com/eugene1g/agent-safehouse"
  url "https://github.com/eugene1g/agent-safehouse/releases/download/v0.9.0/safehouse.sh"
  version "0.9.0"
  sha256 "61c2f71ee13ef9089442cb13cf050cc679e767ec48da9771e7d8f8a3eb2a8697"

  def install
    bin.install "safehouse.sh" => "safehouse"
  end
end
"#,
        )
        .unwrap();

        Mock::given(method("GET"))
            .and(path("/agent-safehouse.json"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        let client = ApiClient::with_base_url(mock_server.uri())
            .with_tap_roots(vec![tap_root.path().to_path_buf()]);
        let formula = client.get_formula("agent-safehouse").await.unwrap();

        assert_eq!(formula.name, "agent-safehouse");
        assert_eq!(formula.versions.stable, "0.9.0");
        assert_eq!(
            formula.ruby_source_path,
            Some(formula_path.display().to_string())
        );
    }

    #[tokio::test]
    async fn resolves_short_name_from_local_tap_after_core_network_failure() {
        let tap_root = tempdir().unwrap();
        let formula_dir = tap_root
            .path()
            .join("eugene1g")
            .join("homebrew-safehouse")
            .join("Formula");
        std::fs::create_dir_all(&formula_dir).unwrap();
        let formula_path = formula_dir.join("agent-safehouse.rb");
        std::fs::write(
            &formula_path,
            r#"
class AgentSafehouse < Formula
  url "https://github.com/eugene1g/agent-safehouse/releases/download/v0.9.0/safehouse.sh"
  version "0.9.0"
  sha256 "61c2f71ee13ef9089442cb13cf050cc679e767ec48da9771e7d8f8a3eb2a8697"

  def install
    bin.install "safehouse.sh" => "safehouse"
  end
end
"#,
        )
        .unwrap();

        let client = ApiClient::with_base_url("http://127.0.0.1:1".to_string())
            .with_tap_roots(vec![tap_root.path().to_path_buf()]);
        let formula = client.get_formula("agent-safehouse").await.unwrap();

        assert_eq!(formula.name, "agent-safehouse");
        assert_eq!(formula.versions.stable, "0.9.0");
        assert_eq!(
            formula.ruby_source_path,
            Some(formula_path.display().to_string())
        );
    }

    #[tokio::test]
    async fn fetch_formula_rb_accepts_local_paths() {
        let source_dir = tempdir().unwrap();
        let cache_dir = tempdir().unwrap();
        let formula_path = source_dir.path().join("foo.rb");
        std::fs::write(&formula_path, "class Foo < Formula\nend\n").unwrap();

        let client = ApiClient::with_base_url("https://example.invalid".to_string());
        let resolved = client
            .fetch_formula_rb(formula_path.to_str().unwrap(), cache_dir.path(), None)
            .await
            .unwrap();

        assert_eq!(resolved, formula_path);
    }

    #[tokio::test]
    async fn first_request_stores_etag() {
        let mock_server = MockServer::start().await;
        let fixture = include_str!("../../fixtures/formula_foo.json");

        Mock::given(method("GET"))
            .and(path("/foo.json"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(fixture)
                    .insert_header("etag", "\"abc123\""),
            )
            .mount(&mock_server)
            .await;

        let cache = ApiCache::in_memory().unwrap();
        let client = ApiClient::with_base_url(mock_server.uri()).with_cache(cache);

        let _ = client.get_formula("foo").await.unwrap();

        let cached = client
            .cache
            .as_ref()
            .unwrap()
            .get(&format!("{}/foo.json", mock_server.uri()))
            .unwrap();
        assert_eq!(cached.etag, Some("\"abc123\"".to_string()));
    }

    #[tokio::test]
    async fn second_request_sends_if_none_match() {
        let mock_server = MockServer::start().await;
        let fixture = include_str!("../../fixtures/formula_foo.json");

        Mock::given(method("GET"))
            .and(path("/foo.json"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(fixture)
                    .insert_header("etag", "\"abc123\""),
            )
            .expect(1)
            .mount(&mock_server)
            .await;

        let cache = ApiCache::in_memory().unwrap();
        let client = ApiClient::with_base_url(mock_server.uri()).with_cache(cache);

        let _ = client.get_formula("foo").await.unwrap();

        mock_server.reset().await;

        Mock::given(method("GET"))
            .and(path("/foo.json"))
            .and(header("If-None-Match", "\"abc123\""))
            .respond_with(ResponseTemplate::new(304))
            .expect(1)
            .mount(&mock_server)
            .await;

        let formula = client.get_formula("foo").await.unwrap();
        assert_eq!(formula.name, "foo");
    }

    #[tokio::test]
    async fn uses_cached_body_on_304() {
        let mock_server = MockServer::start().await;
        let fixture = include_str!("../../fixtures/formula_foo.json");

        Mock::given(method("GET"))
            .and(path("/foo.json"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(fixture)
                    .insert_header("etag", "\"abc123\""),
            )
            .mount(&mock_server)
            .await;

        let cache = ApiCache::in_memory().unwrap();
        let client = ApiClient::with_base_url(mock_server.uri()).with_cache(cache);

        let _ = client.get_formula("foo").await.unwrap();

        mock_server.reset().await;

        Mock::given(method("GET"))
            .and(path("/foo.json"))
            .and(header("If-None-Match", "\"abc123\""))
            .respond_with(ResponseTemplate::new(304))
            .mount(&mock_server)
            .await;

        let formula = client.get_formula("foo").await.unwrap();
        assert_eq!(formula.name, "foo");
        assert_eq!(formula.versions.stable, "1.2.3");
    }

    #[tokio::test]
    async fn fetches_formula_from_tap_ruby_source() {
        let mock_server = MockServer::start().await;
        let rb = r#"
class Terraform < Formula
  version "1.10.0"
  depends_on "go"
  bottle do
    root_url "https://ghcr.io/v2/hashicorp/tap"
    sha256 arm64_sonoma: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  end
end
"#;

        Mock::given(method("GET"))
            .and(path("/hashicorp/homebrew-tap/main/Formula/terraform.rb"))
            .respond_with(ResponseTemplate::new(200).set_body_string(rb))
            .mount(&mock_server)
            .await;

        let client =
            ApiClient::with_base_url(mock_server.uri()).with_tap_raw_base_url(mock_server.uri());
        let formula = client.get_formula("hashicorp/tap/terraform").await.unwrap();

        assert_eq!(formula.name, "terraform");
        assert_eq!(formula.versions.stable, "1.10.0");
        assert!(formula.dependencies.contains(&"go".to_string()));
        assert!(formula.bottle.stable.files.contains_key("arm64_sonoma"));
        let expected_path = format!(
            "{}{}/hashicorp/homebrew-tap/main/Formula/terraform.rb",
            RubySourceLocator::TAP_URL_PREFIX,
            mock_server.uri()
        );
        assert_eq!(
            formula.ruby_source_path.as_deref(),
            Some(expected_path.as_str())
        );
    }

    #[tokio::test]
    async fn supports_source_only_tap_formula_without_bottle_block() {
        let mock_server = MockServer::start().await;
        let rb = r#"
class OhMyPosh < Formula
  version "29.3.0"
  url "https://github.com/JanDeDobbeleer/oh-my-posh/archive/v29.3.0.tar.gz"
  sha256 "ff39f6ef2b4ca2d7d766f2802520b023986a5d6dbcd59fba685a9e5bacf41993"
  depends_on "go@1.26" => :build
end
"#;

        Mock::given(method("GET"))
            .and(path(
                "/jandedobbeleer/homebrew-oh-my-posh/main/oh-my-posh.rb",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string(rb))
            .mount(&mock_server)
            .await;

        let client =
            ApiClient::with_base_url(mock_server.uri()).with_tap_raw_base_url(mock_server.uri());
        let formula = client
            .get_formula("jandedobbeleer/oh-my-posh/oh-my-posh")
            .await
            .unwrap();

        assert_eq!(formula.name, "oh-my-posh");
        assert!(formula.bottle.stable.files.is_empty());
        assert_eq!(formula.build_dependencies, vec!["go@1.26".to_string()]);
        assert!(formula.has_source_url());
        assert!(
            formula
                .ruby_source_path
                .as_deref()
                .is_some_and(|path| path.starts_with(RubySourceLocator::TAP_URL_PREFIX))
        );
    }

    #[tokio::test]
    async fn falls_back_to_master_when_main_missing_for_tap_formula() {
        let mock_server = MockServer::start().await;
        let rb = r#"
class Terraform < Formula
  version "1.10.0"
  bottle do
    root_url "https://ghcr.io/v2/hashicorp/tap"
    sha256 arm64_sonoma: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  end
end
"#;

        Mock::given(method("GET"))
            .and(path("/hashicorp/homebrew-tap/main/Formula/terraform.rb"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/hashicorp/homebrew-tap/master/Formula/terraform.rb"))
            .respond_with(ResponseTemplate::new(200).set_body_string(rb))
            .mount(&mock_server)
            .await;

        let client =
            ApiClient::with_base_url(mock_server.uri()).with_tap_raw_base_url(mock_server.uri());
        let formula = client.get_formula("hashicorp/tap/terraform").await.unwrap();

        assert_eq!(formula.name, "terraform");
        assert_eq!(formula.versions.stable, "1.10.0");
    }

    #[tokio::test]
    async fn resolves_tap_formula_from_letter_subdirectory_path() {
        let mock_server = MockServer::start().await;
        let rb = r#"
class Terraform < Formula
  version "1.10.0"
  bottle do
    root_url "https://ghcr.io/v2/hashicorp/tap"
    sha256 arm64_sonoma: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  end
end
"#;

        Mock::given(method("GET"))
            .and(path("/hashicorp/homebrew-tap/main/Formula/t/terraform.rb"))
            .respond_with(ResponseTemplate::new(200).set_body_string(rb))
            .mount(&mock_server)
            .await;

        let client =
            ApiClient::with_base_url(mock_server.uri()).with_tap_raw_base_url(mock_server.uri());
        let formula = client.get_formula("hashicorp/tap/terraform").await.unwrap();

        assert_eq!(formula.name, "terraform");
        assert_eq!(formula.versions.stable, "1.10.0");
    }

    #[tokio::test]
    async fn resolves_tap_formula_from_homebrewformula_directory() {
        let mock_server = MockServer::start().await;
        let rb = r#"
class Terraform < Formula
  version "1.10.0"
  bottle do
    root_url "https://ghcr.io/v2/hashicorp/tap"
    sha256 arm64_sonoma: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  end
end
"#;

        Mock::given(method("GET"))
            .and(path(
                "/hashicorp/homebrew-tap/main/HomebrewFormula/terraform.rb",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string(rb))
            .mount(&mock_server)
            .await;

        let client =
            ApiClient::with_base_url(mock_server.uri()).with_tap_raw_base_url(mock_server.uri());
        let formula = client.get_formula("hashicorp/tap/terraform").await.unwrap();

        assert_eq!(formula.name, "terraform");
        assert_eq!(formula.versions.stable, "1.10.0");
    }

    #[tokio::test]
    async fn resolves_tap_formula_from_homebrewformula_letter_subdirectory_path() {
        let mock_server = MockServer::start().await;
        let rb = r#"
class Terraform < Formula
  version "1.10.0"
  bottle do
    root_url "https://ghcr.io/v2/hashicorp/tap"
    sha256 arm64_sonoma: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  end
end
"#;

        Mock::given(method("GET"))
            .and(path(
                "/hashicorp/homebrew-tap/main/HomebrewFormula/t/terraform.rb",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string(rb))
            .mount(&mock_server)
            .await;

        let client =
            ApiClient::with_base_url(mock_server.uri()).with_tap_raw_base_url(mock_server.uri());
        let formula = client.get_formula("hashicorp/tap/terraform").await.unwrap();

        assert_eq!(formula.name, "terraform");
        assert_eq!(formula.versions.stable, "1.10.0");
    }

    #[tokio::test]
    async fn resolves_tap_formula_from_repository_root() {
        let mock_server = MockServer::start().await;
        let rb = r#"
class Terraform < Formula
  version "1.10.0"
  bottle do
    root_url "https://ghcr.io/v2/hashicorp/tap"
    sha256 arm64_sonoma: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  end
end
"#;

        Mock::given(method("GET"))
            .and(path("/hashicorp/homebrew-tap/main/terraform.rb"))
            .respond_with(ResponseTemplate::new(200).set_body_string(rb))
            .mount(&mock_server)
            .await;

        let client =
            ApiClient::with_base_url(mock_server.uri()).with_tap_raw_base_url(mock_server.uri());
        let formula = client.get_formula("hashicorp/tap/terraform").await.unwrap();

        assert_eq!(formula.name, "terraform");
        assert_eq!(formula.versions.stable, "1.10.0");
    }

    #[tokio::test]
    async fn returns_missing_formula_when_all_tap_candidates_are_404() {
        let mock_server = MockServer::start().await;

        let client =
            ApiClient::with_base_url(mock_server.uri()).with_tap_raw_base_url(mock_server.uri());
        let err = client
            .get_formula("hashicorp/tap/terraform")
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            Error::MissingFormula { name } if name == "hashicorp/tap/terraform"
        ));
    }

    #[tokio::test]
    async fn does_not_return_missing_formula_when_a_non_404_tap_status_is_seen() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/hashicorp/homebrew-tap/main/Formula/terraform.rb"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&mock_server)
            .await;

        let client =
            ApiClient::with_base_url(mock_server.uri()).with_tap_raw_base_url(mock_server.uri());
        let err = client
            .get_formula("hashicorp/tap/terraform")
            .await
            .unwrap_err();

        assert!(matches!(err, Error::NetworkFailure { .. }));
    }

    #[tokio::test]
    async fn fetch_formula_rb_supports_absolute_url_paths() {
        let mock_server = MockServer::start().await;
        let ruby_body = "class Foo < Formula\nend\n";

        Mock::given(method("GET"))
            .and(path("/custom/foo.rb"))
            .respond_with(ResponseTemplate::new(200).set_body_string(ruby_body))
            .mount(&mock_server)
            .await;

        let cache_dir = tempdir().unwrap();
        let client = ApiClient::new();

        let fetched = client
            .fetch_formula_rb(
                &format!("{}/custom/foo.rb", mock_server.uri()),
                cache_dir.path(),
                None,
            )
            .await
            .unwrap();

        assert!(fetched.exists());
    }

    #[tokio::test]
    async fn fetch_formula_rb_from_network_rejects_checksum_mismatch() {
        let mock_server = MockServer::start().await;
        let ruby_body = "class Foo < Formula\nend\n";

        Mock::given(method("GET"))
            .and(path("/Formula/f/foo.rb"))
            .respond_with(ResponseTemplate::new(200).set_body_string(ruby_body))
            .mount(&mock_server)
            .await;

        let cache_dir = tempdir().unwrap();
        let client = ApiClient::new();

        let err = client
            .fetch_formula_rb_from_url(
                "Formula/f/foo.rb",
                &format!("{}/Formula/f/foo.rb", mock_server.uri()),
                cache_dir.path(),
                Some(&"0".repeat(64)),
            )
            .await
            .unwrap_err();

        assert!(matches!(err, Error::ChecksumMismatch { .. }));
    }

    #[tokio::test]
    async fn fetch_formula_rb_from_cache_rejects_checksum_mismatch() {
        let cache = ApiCache::in_memory().unwrap();
        let cache_url = "https://example.invalid/Formula/f/foo.rb";
        cache
            .put(
                &format!("rb:{cache_url}"),
                &CacheEntry {
                    etag: None,
                    last_modified: None,
                    body: "class Foo < Formula\nend\n".to_string(),
                },
            )
            .unwrap();

        let cache_dir = tempdir().unwrap();
        let client = ApiClient::new().with_cache(cache);

        let err = client
            .fetch_formula_rb_from_url(
                "Formula/f/foo.rb",
                cache_url,
                cache_dir.path(),
                Some(&"f".repeat(64)),
            )
            .await
            .unwrap_err();

        assert!(matches!(err, Error::ChecksumMismatch { .. }));
    }

    #[tokio::test]
    async fn fetches_cask_json() {
        let mock_server = MockServer::start().await;
        let cask_json = r#"{
  "token": "iterm2",
  "version": "3.5.0",
  "url": "https://example.com/iterm2.zip",
  "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "artifacts": [{"app":["iTerm.app"]}]
}"#;

        Mock::given(method("GET"))
            .and(path("/iterm2.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(cask_json))
            .mount(&mock_server)
            .await;

        let client =
            ApiClient::with_base_url(mock_server.uri()).with_cask_base_url(mock_server.uri());
        let cask = client.get_cask("iterm2").await.unwrap();
        assert_eq!(cask["token"], "iterm2");
        assert_eq!(cask["version"], "3.5.0");
    }
}
