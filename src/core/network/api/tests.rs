use super::*;
use std::assert_matches;
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

    let fixture = include_str!("../../../fixtures/formula_foo.json");

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

    assert_matches!(
        err,
        Error::MissingFormula { name } if name == "nonexistent"
    );
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
    let fixture = include_str!("../../../fixtures/formula_foo.json");

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
    let fixture = include_str!("../../../fixtures/formula_foo.json");

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
    let fixture = include_str!("../../../fixtures/formula_foo.json");

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

    assert_matches!(
        err,
        Error::MissingFormula { name } if name == "hashicorp/tap/terraform"
    );
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

    assert_matches!(err, Error::NetworkFailure { .. });
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

    Mock::given(method("GET"))
        .and(path("/iterm2.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(cask_json))
        .mount(&mock_server)
        .await;

    let client = ApiClient::with_base_url(mock_server.uri()).with_cask_base_url(mock_server.uri());
    let cask = client.get_cask("iterm2").await.unwrap();
    assert_eq!(cask["token"], "iterm2");
    assert_eq!(cask["version"], "3.5.0");
}

fn write_stale_meta(cache_dir: &std::path::Path, slug: &str, etag: Option<&str>) {
    let meta = IndexMeta {
        etag: etag.map(|s| s.to_string()),
        last_modified: Some("Wed, 01 Jan 2020 00:00:00 GMT".to_string()),
        fetched_at: 0,
    };
    std::fs::write(
        cache_dir.join(format!("{slug}.meta.json")),
        serde_json::to_string(&meta).unwrap(),
    )
    .unwrap();
}

#[tokio::test]
async fn fetch_index_writes_body_and_meta_on_fresh_200() {
    let mock_server = MockServer::start().await;
    let cache_dir = tempdir().unwrap();

    Mock::given(method("GET"))
        .and(path("/formula.json"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("[]")
                .insert_header("etag", "\"abc123\"")
                .insert_header("last-modified", "Wed, 21 Oct 2026 07:28:00 GMT"),
        )
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = ApiClient::new();
    let url = format!("{}/formula.json", mock_server.uri());
    let body = client
        .fetch_index(&url, cache_dir.path(), "formula", false)
        .await
        .unwrap();

    assert_eq!(body, "[]");
    assert!(cache_dir.path().join("formula.json").exists());
    let meta_raw = std::fs::read_to_string(cache_dir.path().join("formula.meta.json")).unwrap();
    let meta: IndexMeta = serde_json::from_str(&meta_raw).unwrap();
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

    std::fs::write(cache_dir.path().join("formula.json"), "[\"cached\"]").unwrap();
    let meta = IndexMeta {
        etag: Some("\"old\"".to_string()),
        last_modified: None,
        fetched_at: now_secs(),
    };
    std::fs::write(
        cache_dir.path().join("formula.meta.json"),
        serde_json::to_string(&meta).unwrap(),
    )
    .unwrap();

    Mock::given(method("GET"))
        .and(path("/formula.json"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&mock_server)
        .await;

    let client = ApiClient::new();
    let url = format!("{}/formula.json", mock_server.uri());
    let body = client
        .fetch_index(&url, cache_dir.path(), "formula", false)
        .await
        .unwrap();

    assert_eq!(body, "[\"cached\"]");
}

#[tokio::test]
async fn fetch_index_revalidates_with_304_when_stale() {
    let mock_server = MockServer::start().await;
    let cache_dir = tempdir().unwrap();

    std::fs::write(cache_dir.path().join("formula.json"), "[\"cached\"]").unwrap();
    write_stale_meta(cache_dir.path(), "formula", Some("\"etag-v1\""));

    Mock::given(method("GET"))
        .and(path("/formula.json"))
        .and(header("if-none-match", "\"etag-v1\""))
        .respond_with(ResponseTemplate::new(304))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = ApiClient::new();
    let url = format!("{}/formula.json", mock_server.uri());
    let body = client
        .fetch_index(&url, cache_dir.path(), "formula", false)
        .await
        .unwrap();

    assert_eq!(body, "[\"cached\"]");
    let meta_raw = std::fs::read_to_string(cache_dir.path().join("formula.meta.json")).unwrap();
    let meta: IndexMeta = serde_json::from_str(&meta_raw).unwrap();
    assert!(
        meta.fetched_at > 0,
        "fetched_at should be refreshed after 304"
    );
}

#[tokio::test]
async fn fetch_index_refresh_forces_revalidation_even_when_fresh() {
    let mock_server = MockServer::start().await;
    let cache_dir = tempdir().unwrap();

    std::fs::write(cache_dir.path().join("formula.json"), "[\"old\"]").unwrap();
    let meta = IndexMeta {
        etag: Some("\"etag-v1\"".to_string()),
        last_modified: None,
        fetched_at: now_secs(),
    };
    std::fs::write(
        cache_dir.path().join("formula.meta.json"),
        serde_json::to_string(&meta).unwrap(),
    )
    .unwrap();

    Mock::given(method("GET"))
        .and(path("/formula.json"))
        .and(header("if-none-match", "\"etag-v1\""))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("[\"new\"]")
                .insert_header("etag", "\"etag-v2\""),
        )
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = ApiClient::new();
    let url = format!("{}/formula.json", mock_server.uri());
    let body = client
        .fetch_index(&url, cache_dir.path(), "formula", true)
        .await
        .unwrap();

    assert_eq!(body, "[\"new\"]");
    let meta_raw = std::fs::read_to_string(cache_dir.path().join("formula.meta.json")).unwrap();
    let meta: IndexMeta = serde_json::from_str(&meta_raw).unwrap();
    assert_eq!(meta.etag.as_deref(), Some("\"etag-v2\""));
}

#[tokio::test]
async fn fetch_index_returns_stale_on_server_error() {
    let mock_server = MockServer::start().await;
    let cache_dir = tempdir().unwrap();

    std::fs::write(cache_dir.path().join("formula.json"), "[\"cached\"]").unwrap();
    write_stale_meta(cache_dir.path(), "formula", None);

    Mock::given(method("GET"))
        .and(path("/formula.json"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&mock_server)
        .await;

    let client = ApiClient::new();
    let url = format!("{}/formula.json", mock_server.uri());
    let body = client
        .fetch_index(&url, cache_dir.path(), "formula", false)
        .await
        .unwrap();

    assert_eq!(body, "[\"cached\"]");
}

#[tokio::test]
async fn fetch_index_server_error_without_cache_returns_network_error() {
    let mock_server = MockServer::start().await;
    let cache_dir = tempdir().unwrap();

    Mock::given(method("GET"))
        .and(path("/formula.json"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&mock_server)
        .await;

    let client = ApiClient::new();
    let url = format!("{}/formula.json", mock_server.uri());
    let err = client
        .fetch_index(&url, cache_dir.path(), "formula", false)
        .await
        .unwrap_err();

    assert_matches!(err, Error::NetworkFailure { .. });
}

#[tokio::test]
async fn fetch_index_stale_on_network_failure() {
    let cache_dir = tempdir().unwrap();
    std::fs::write(cache_dir.path().join("formula.json"), "[\"cached\"]").unwrap();
    write_stale_meta(cache_dir.path(), "formula", None);

    // Point at an unreachable port to force a connection failure.
    let client = ApiClient::new();
    let url = "http://127.0.0.1:1/formula.json";
    let body = client
        .fetch_index(url, cache_dir.path(), "formula", false)
        .await
        .unwrap();

    assert_eq!(body, "[\"cached\"]");
}
