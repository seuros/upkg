use super::*;
use std::assert_matches;
use std::path::{Path, PathBuf};
use tempfile::tempdir;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockBuilder, MockServer, ResponseTemplate};

// MOCKED: wiremock stands in for the formula API, tap raw hosts, and index URLs.
const FOO_FIXTURE: &str = include_str!("../../../fixtures/formula_foo.json");
const FOO_ETAG: &str = "\"abc123\"";
const TERRAFORM_TAP_RB: &str = r#"
class Terraform < Formula
  version "1.10.0"
  bottle do
    root_url "https://ghcr.io/v2/hashicorp/tap"
    sha256 arm64_sonoma: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  end
end
"#;

fn get(route: &str) -> MockBuilder {
    Mock::given(method("GET")).and(path(route))
}

async fn mount_get(server: &MockServer, route: &str, response: ResponseTemplate) {
    get(route).respond_with(response).mount(server).await;
}

fn tap_client(server: &MockServer) -> ApiClient {
    ApiClient::with_base_url(server.uri()).with_tap_raw_base_url(server.uri())
}

/// Writes `eugene1g/homebrew-safehouse/Formula/agent-safehouse.rb` under `tap_root`.
fn write_local_tap_formula(tap_root: &Path, body: &str) -> PathBuf {
    let formula_dir = tap_root
        .join("eugene1g")
        .join("homebrew-safehouse")
        .join("Formula");
    std::fs::create_dir_all(&formula_dir).unwrap();
    let formula_path = formula_dir.join("agent-safehouse.rb");
    std::fs::write(&formula_path, body).unwrap();
    formula_path
}

/// `/foo.json` answering 200 with the fixture body and an `ETag`.
fn foo_with_etag() -> Mock {
    get("/foo.json").respond_with(
        ResponseTemplate::new(200)
            .set_body_string(FOO_FIXTURE)
            .insert_header("etag", FOO_ETAG),
    )
}

/// `/foo.json` answering 304 only when revalidated with the stored `ETag`.
fn foo_not_modified() -> Mock {
    get("/foo.json")
        .and(header("If-None-Match", FOO_ETAG))
        .respond_with(ResponseTemplate::new(304))
}

/// Client with an in-memory cache, primed by one successful `get_formula("foo")`.
async fn primed_cached_client(server: &MockServer) -> ApiClient {
    let client = ApiClient::with_base_url(server.uri()).with_cache(ApiCache::in_memory().unwrap());
    client.get_formula("foo").await.unwrap();
    client
}

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
        RubySourceLocator::LocalPath("/opt/homebrew/Library/Taps/me/homebrew-tools/Formula/foo.rb")
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
        RubySourceLocator::TapEncodedUrl("https://raw.githubusercontent.com/org/tap/main/foo.rb")
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
    mount_get(
        &mock_server,
        "/foo.json",
        ResponseTemplate::new(200).set_body_string(FOO_FIXTURE),
    )
    .await;

    let client = ApiClient::with_base_url(mock_server.uri());
    let formula = client.get_formula("foo").await.unwrap();

    assert_eq!(formula.name, "foo");
    assert_eq!(formula.versions.stable, "1.2.3");
}

#[tokio::test]
async fn returns_missing_formula_on_404() {
    let mock_server = MockServer::start().await;
    mount_get(
        &mock_server,
        "/nonexistent.json",
        ResponseTemplate::new(404),
    )
    .await;

    let client = ApiClient::with_base_url(mock_server.uri());
    let err = client.get_formula("nonexistent").await.unwrap_err();

    assert_matches!(
        err,
        Error::MissingFormula { name } if name == "nonexistent"
    );
}

#[tokio::test]
async fn resolves_short_name_from_local_tap_after_core_404() {
    let mock_server = MockServer::start().await;
    let tap_root = tempdir().unwrap();
    let formula_path = write_local_tap_formula(
        tap_root.path(),
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
    );
    mount_get(
        &mock_server,
        "/agent-safehouse.json",
        ResponseTemplate::new(404),
    )
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
    let formula_path = write_local_tap_formula(
        tap_root.path(),
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
    );

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
    foo_with_etag().mount(&mock_server).await;

    let client = primed_cached_client(&mock_server).await;

    let cached = client
        .cache
        .as_ref()
        .unwrap()
        .get(&format!("{}/foo.json", mock_server.uri()))
        .unwrap();
    assert_eq!(cached.etag.as_deref(), Some(FOO_ETAG));
}

#[tokio::test]
async fn second_request_sends_if_none_match() {
    let mock_server = MockServer::start().await;
    foo_with_etag().expect(1).mount(&mock_server).await;

    let client = primed_cached_client(&mock_server).await;
    mock_server.reset().await;
    foo_not_modified().expect(1).mount(&mock_server).await;

    let formula = client.get_formula("foo").await.unwrap();
    assert_eq!(formula.name, "foo");
}

#[tokio::test]
async fn uses_cached_body_on_304() {
    let mock_server = MockServer::start().await;
    foo_with_etag().mount(&mock_server).await;

    let client = primed_cached_client(&mock_server).await;
    mock_server.reset().await;
    foo_not_modified().mount(&mock_server).await;

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
    mount_get(
        &mock_server,
        "/hashicorp/homebrew-tap/main/Formula/terraform.rb",
        ResponseTemplate::new(200).set_body_string(rb),
    )
    .await;

    let formula = tap_client(&mock_server)
        .get_formula("hashicorp/tap/terraform")
        .await
        .unwrap();

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
    mount_get(
        &mock_server,
        "/jandedobbeleer/homebrew-oh-my-posh/main/oh-my-posh.rb",
        ResponseTemplate::new(200).set_body_string(rb),
    )
    .await;

    let formula = tap_client(&mock_server)
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
    mount_get(
        &mock_server,
        "/hashicorp/homebrew-tap/main/Formula/terraform.rb",
        ResponseTemplate::new(404),
    )
    .await;
    mount_get(
        &mock_server,
        "/hashicorp/homebrew-tap/master/Formula/terraform.rb",
        ResponseTemplate::new(200).set_body_string(TERRAFORM_TAP_RB),
    )
    .await;

    let formula = tap_client(&mock_server)
        .get_formula("hashicorp/tap/terraform")
        .await
        .unwrap();

    assert_eq!(formula.name, "terraform");
    assert_eq!(formula.versions.stable, "1.10.0");
}

#[tokio::test]
async fn resolves_tap_formula_from_candidate_paths() {
    const CANDIDATES: &[&str] = &[
        // letter subdirectory
        "/hashicorp/homebrew-tap/main/Formula/t/terraform.rb",
        // HomebrewFormula directory
        "/hashicorp/homebrew-tap/main/HomebrewFormula/terraform.rb",
        // HomebrewFormula letter subdirectory
        "/hashicorp/homebrew-tap/main/HomebrewFormula/t/terraform.rb",
        // repository root
        "/hashicorp/homebrew-tap/main/terraform.rb",
    ];

    for route in CANDIDATES {
        let mock_server = MockServer::start().await;
        mount_get(
            &mock_server,
            route,
            ResponseTemplate::new(200).set_body_string(TERRAFORM_TAP_RB),
        )
        .await;

        let formula = tap_client(&mock_server)
            .get_formula("hashicorp/tap/terraform")
            .await
            .unwrap();

        assert_eq!(formula.name, "terraform", "route: {route}");
        assert_eq!(formula.versions.stable, "1.10.0", "route: {route}");
    }
}

#[tokio::test]
async fn returns_missing_formula_when_all_tap_candidates_are_404() {
    let mock_server = MockServer::start().await;

    let err = tap_client(&mock_server)
        .get_formula("hashicorp/tap/terraform")
        .await
        .unwrap_err();

    assert_matches!(
        err,
        Error::MissingFormula { name } if name == "hashicorp/tap/terraform"
    );
}

#[tokio::test]
async fn does_not_return_missing_formula_when_a_non_404_tap_status_is_seen() {
    let mock_server = MockServer::start().await;
    mount_get(
        &mock_server,
        "/hashicorp/homebrew-tap/main/Formula/terraform.rb",
        ResponseTemplate::new(500),
    )
    .await;

    let err = tap_client(&mock_server)
        .get_formula("hashicorp/tap/terraform")
        .await
        .unwrap_err();

    assert_matches!(err, Error::NetworkFailure { .. });
}

#[tokio::test]
async fn fetch_formula_rb_supports_absolute_url_paths() {
    let mock_server = MockServer::start().await;
    mount_get(
        &mock_server,
        "/custom/foo.rb",
        ResponseTemplate::new(200).set_body_string("class Foo < Formula\nend\n"),
    )
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
    mount_get(
        &mock_server,
        "/Formula/f/foo.rb",
        ResponseTemplate::new(200).set_body_string("class Foo < Formula\nend\n"),
    )
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

    assert_matches!(err, Error::ChecksumMismatch { .. });
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

    assert_matches!(err, Error::ChecksumMismatch { .. });
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
    mount_get(
        &mock_server,
        "/iterm2.json",
        ResponseTemplate::new(200).set_body_string(cask_json),
    )
    .await;

    let client = ApiClient::with_base_url(mock_server.uri()).with_cask_base_url(mock_server.uri());
    let cask = client.get_cask("iterm2").await.unwrap();
    assert_eq!(cask["token"], "iterm2");
    assert_eq!(cask["version"], "3.5.0");
}

/// Seeds `formula.json` with `body` and `formula.meta.json` with `meta`.
fn write_cached_index(cache_dir: &Path, body: &str, meta: &IndexMeta) {
    std::fs::write(cache_dir.join("formula.json"), body).unwrap();
    std::fs::write(
        cache_dir.join("formula.meta.json"),
        serde_json::to_string(meta).unwrap(),
    )
    .unwrap();
}

/// Meta fetched just now, so it is within the freshness window.
fn fresh_meta(etag: &str) -> IndexMeta {
    IndexMeta {
        etag: Some(etag.to_string()),
        last_modified: None,
        fetched_at: now_secs(),
    }
}

/// Meta fetched at the epoch, so it always needs revalidation.
fn stale_meta(etag: Option<&str>) -> IndexMeta {
    IndexMeta {
        etag: etag.map(str::to_string),
        last_modified: Some("Wed, 01 Jan 2020 00:00:00 GMT".to_string()),
        fetched_at: 0,
    }
}

fn read_index_meta(cache_dir: &Path) -> IndexMeta {
    let raw = std::fs::read_to_string(cache_dir.join("formula.meta.json")).unwrap();
    serde_json::from_str(&raw).unwrap()
}

async fn fetch_formula_index(
    server: &MockServer,
    cache_dir: &Path,
    refresh: bool,
) -> Result<String, Error> {
    let url = format!("{}/formula.json", server.uri());
    ApiClient::new()
        .fetch_index(&url, cache_dir, "formula", refresh)
        .await
}

#[tokio::test]
async fn fetch_index_writes_body_and_meta_on_fresh_200() {
    let mock_server = MockServer::start().await;
    let cache_dir = tempdir().unwrap();

    get("/formula.json")
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("[]")
                .insert_header("etag", "\"abc123\"")
                .insert_header("last-modified", "Wed, 21 Oct 2026 07:28:00 GMT"),
        )
        .expect(1)
        .mount(&mock_server)
        .await;

    let body = fetch_formula_index(&mock_server, cache_dir.path(), false)
        .await
        .unwrap();

    assert_eq!(body, "[]");
    assert!(cache_dir.path().join("formula.json").exists());
    let meta = read_index_meta(cache_dir.path());
    assert_eq!(meta.etag.as_deref(), Some("\"abc123\""));
    assert_eq!(
        meta.last_modified.as_deref(),
        Some("Wed, 21 Oct 2026 07:28:00 GMT")
    );
    assert!(meta.fetched_at > 0);
}

#[tokio::test]
async fn fetch_index_uses_cache_when_fresh_no_network() {
    let mock_server = MockServer::start().await;
    let cache_dir = tempdir().unwrap();
    write_cached_index(cache_dir.path(), "[\"cached\"]", &fresh_meta("\"old\""));

    get("/formula.json")
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&mock_server)
        .await;

    let body = fetch_formula_index(&mock_server, cache_dir.path(), false)
        .await
        .unwrap();

    assert_eq!(body, "[\"cached\"]");
}

#[tokio::test]
async fn fetch_index_revalidates_with_304_when_stale() {
    let mock_server = MockServer::start().await;
    let cache_dir = tempdir().unwrap();
    write_cached_index(
        cache_dir.path(),
        "[\"cached\"]",
        &stale_meta(Some("\"etag-v1\"")),
    );

    get("/formula.json")
        .and(header("if-none-match", "\"etag-v1\""))
        .respond_with(ResponseTemplate::new(304))
        .expect(1)
        .mount(&mock_server)
        .await;

    let body = fetch_formula_index(&mock_server, cache_dir.path(), false)
        .await
        .unwrap();

    assert_eq!(body, "[\"cached\"]");
    assert!(
        read_index_meta(cache_dir.path()).fetched_at > 0,
        "fetched_at should be refreshed after 304"
    );
}

#[tokio::test]
async fn fetch_index_refresh_forces_revalidation_even_when_fresh() {
    let mock_server = MockServer::start().await;
    let cache_dir = tempdir().unwrap();
    write_cached_index(cache_dir.path(), "[\"old\"]", &fresh_meta("\"etag-v1\""));

    get("/formula.json")
        .and(header("if-none-match", "\"etag-v1\""))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("[\"new\"]")
                .insert_header("etag", "\"etag-v2\""),
        )
        .expect(1)
        .mount(&mock_server)
        .await;

    let body = fetch_formula_index(&mock_server, cache_dir.path(), true)
        .await
        .unwrap();

    assert_eq!(body, "[\"new\"]");
    assert_eq!(
        read_index_meta(cache_dir.path()).etag.as_deref(),
        Some("\"etag-v2\"")
    );
}

#[tokio::test]
async fn fetch_index_server_error_falls_back_to_stale_cache_or_fails() {
    // (has stale cache, expected body) — `None` means a network error is expected.
    const CASES: &[(bool, Option<&str>)] = &[(true, Some("[\"cached\"]")), (false, None)];

    for &(seed_cache, expected) in CASES {
        let mock_server = MockServer::start().await;
        let cache_dir = tempdir().unwrap();
        if seed_cache {
            write_cached_index(cache_dir.path(), "[\"cached\"]", &stale_meta(None));
        }
        mount_get(&mock_server, "/formula.json", ResponseTemplate::new(503)).await;

        let result = fetch_formula_index(&mock_server, cache_dir.path(), false).await;

        match expected {
            Some(body) => assert_eq!(result.unwrap(), body, "seed_cache: {seed_cache}"),
            None => assert_matches!(result, Err(Error::NetworkFailure { .. })),
        }
    }
}

#[tokio::test]
async fn fetch_index_stale_on_network_failure() {
    let cache_dir = tempdir().unwrap();
    write_cached_index(cache_dir.path(), "[\"cached\"]", &stale_meta(None));

    // Point at an unreachable port to force a connection failure.
    let body = ApiClient::new()
        .fetch_index(
            "http://127.0.0.1:1/formula.json",
            cache_dir.path(),
            "formula",
            false,
        )
        .await
        .unwrap();

    assert_eq!(body, "[\"cached\"]");
}
