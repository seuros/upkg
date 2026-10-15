use super::*;
use std::assert_matches;

fn parse(owner: &str, repo: &str, formula: &str, source: &str) -> Result<Formula, Error> {
    let spec = TapFormulaRef {
        owner: owner.to_string(),
        repo: repo.to_string(),
        formula: formula.to_string(),
    };
    parse_tap_formula_ruby(&spec, source)
}

fn assert_stable_source(formula: &Formula, url: &str, checksum: &str) {
    let stable = formula
        .urls
        .as_ref()
        .and_then(|u| u.stable.as_ref())
        .expect("stable source url should be parsed");
    assert_eq!(stable.url, url);
    assert_eq!(stable.checksum.as_deref(), Some(checksum));
}

#[test]
fn parses_tap_formula_reference() {
    let parsed = parse_tap_formula_ref("hashicorp/tap/terraform").unwrap();
    assert_eq!(parsed.owner, "hashicorp");
    assert_eq!(parsed.repo, "tap");
    assert_eq!(parsed.formula, "terraform");
}

#[test]
fn rejects_non_tap_reference() {
    for input in ["jq", "a/b", "a/b/c/d"] {
        assert!(parse_tap_formula_ref(input).is_none(), "input: {input:?}");
    }
}

#[test]
fn parses_formula_subset_with_bottle_data() {
    let source = r#"
class Terraform < Formula
  version "1.10.0"
  revision 1
  depends_on "go" => :build
  depends_on "openssl@3"

  bottle do
    root_url "https://ghcr.io/v2/hashicorp/tap"
    rebuild 2
    sha256 cellar: :any_skip_relocation, arm64_sonoma: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    sha256 cellar: :any_skip_relocation, x86_64_linux: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
  end
end
"#;

    let formula = parse("hashicorp", "tap", "terraform", source).unwrap();
    assert_eq!(formula.name, "terraform");
    assert_eq!(formula.versions.stable, "1.10.0");
    assert_eq!(formula.revision, 1);
    assert_eq!(formula.bottle.stable.rebuild, 2);
    assert_eq!(formula.dependencies, vec!["openssl@3".to_string()]);
    assert_eq!(formula.build_dependencies, vec!["go".to_string()]);
    assert!(formula.bottle.stable.files.contains_key("arm64_sonoma"));
    assert!(formula.bottle.stable.files.contains_key("x86_64_linux"));
}

#[test]
fn defaults_to_ghcr_root_url_when_missing() {
    let source = r#"
class Terraform < Formula
  bottle do
    sha256 arm64_sonoma: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  end
end
"#;

    let formula = parse("hashicorp", "tap", "terraform", source).unwrap();
    assert_eq!(
        formula.bottle.stable.files["arm64_sonoma"].url,
        "https://ghcr.io/v2/hashicorp/tap/terraform/blobs/sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
}

#[test]
fn builds_release_style_bottle_url() {
    let source = r#"
class Ttfb < Formula
  version "1.3.0"
  bottle do
    root_url "https://github.com/messense/homebrew-tap/releases/download/ttfb-1.3.0"
    sha256 x86_64_linux: "054859a821b01d3dd7236e71fbf106f7a694ded54ae6aaaed221b59d3b554c42"
  end
end
"#;
    let formula = parse("messense", "tap", "ttfb", source).unwrap();
    assert_eq!(
        formula.bottle.stable.files["x86_64_linux"].url,
        "https://github.com/messense/homebrew-tap/releases/download/ttfb-1.3.0/ttfb-1.3.0.x86_64_linux.bottle.tar.gz"
    );
}

#[test]
fn infers_version_from_url_when_version_field_missing() {
    let source = r#"
class Jaso < Formula
  url "https://github.com/cr0sh/jaso/archive/refs/tags/v1.0.1.tar.gz"
  bottle do
    root_url "https://github.com/simnalamburt/homebrew-x/releases/download/jaso-1.0.1"
    sha256 x86_64_linux: "76c0ea0751627a7aac5495c460eecd8a7823c86e5e55b078b5884056efa8ae7f"
  end
end
"#;
    let formula = parse("simnalamburt", "x", "jaso", source).unwrap();
    assert_eq!(formula.versions.stable, "1.0.1");
    assert_eq!(
        formula.bottle.stable.files["x86_64_linux"].url,
        "https://github.com/simnalamburt/homebrew-x/releases/download/jaso-1.0.1/jaso-1.0.1.x86_64_linux.bottle.tar.gz"
    );
}

#[test]
fn parses_bottle_block_with_nested_do_end_sections() {
    let source = r#"
class Terraform < Formula
  version "1.10.0"
  bottle do
    root_url "https://ghcr.io/v2/hashicorp/tap"
    on_linux do
      sha256 x86_64_linux: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    end
    on_macos do
      sha256 arm64_sonoma: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    end
  end
end
"#;

    let formula = parse("hashicorp", "tap", "terraform", source).unwrap();

    assert!(formula.bottle.stable.files.contains_key("x86_64_linux"));
    assert!(formula.bottle.stable.files.contains_key("arm64_sonoma"));
}

#[test]
fn supports_source_only_tap_formula_without_bottle_block() {
    let source = r#"
class OhMyPosh < Formula
  version "29.3.0"
  url "https://github.com/JanDeDobbeleer/oh-my-posh/archive/v29.3.0.tar.gz"
  sha256 "ff39f6ef2b4ca2d7d766f2802520b023986a5d6dbcd59fba685a9e5bacf41993"
  depends_on "go@1.26" => :build
end
"#;

    let formula = parse("jandedobbeleer", "oh-my-posh", "oh-my-posh", source).unwrap();
    assert!(formula.bottle.stable.files.is_empty());
    assert_eq!(formula.build_dependencies, vec!["go@1.26".to_string()]);
    assert_stable_source(
        &formula,
        "https://github.com/JanDeDobbeleer/oh-my-posh/archive/v29.3.0.tar.gz",
        "ff39f6ef2b4ca2d7d766f2802520b023986a5d6dbcd59fba685a9e5bacf41993",
    );
}

#[test]
fn source_url_parsing_ignores_nested_resource_blocks() {
    let source = r#"
class Example < Formula
  url "https://example.com/example-1.0.0.tar.gz"
  sha256 "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"

  resource "extra" do
    url "https://example.com/resource.tar.gz"
    sha256 "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
  end
end
"#;

    let formula = parse("someone", "tap", "example", source).unwrap();
    assert_stable_source(
        &formula,
        "https://example.com/example-1.0.0.tar.gz",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    );
}

#[test]
fn source_url_without_top_level_sha256_is_unsupported() {
    const CASES: &[(&str, &str)] = &[
        (
            "no sha256 at all",
            r#"
class Example < Formula
  url "https://example.com/example-1.0.0.tar.gz"
end
"#,
        ),
        (
            "sha256 only inside a nested resource",
            r#"
class Example < Formula
  url "https://example.com/example-1.0.0.tar.gz"

  resource "extra" do
    url "https://example.com/resource.tar.gz"
    sha256 "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
  end
end
"#,
        ),
    ];

    for (case, source) in CASES {
        let err = parse("someone", "tap", "example", source).unwrap_err();
        assert_matches!(
            err,
            Error::UnsupportedFormula { ref reason, .. } if reason.contains("missing sha256"),
            "case: {case}"
        );
    }
}

#[test]
fn dependency_parsing_ignores_nested_blocks() {
    let source = r#"
class Example < Formula
  url "https://example.com/example-1.0.0.tar.gz"
  sha256 "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  depends_on "openssl@3"
  depends_on "go" => :build

  resource "extra" do
    depends_on "python@3.12"
  end
end
"#;

    let formula = parse("someone", "tap", "example", source).unwrap();
    assert_eq!(formula.dependencies, vec!["openssl@3".to_string()]);
    assert_eq!(formula.build_dependencies, vec!["go".to_string()]);
}

#[test]
fn parser_does_not_treat_do_inside_strings_as_block_start() {
    let source = r#"
class Example < Formula
  desc "A tool to do amazing things"
  url "https://example.com/example-1.0.0.tar.gz"
  sha256 "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  depends_on "openssl@3"
  depends_on "go" => :build

  resource "extra" do |r|
    depends_on "python@3.12"
    r.url "https://example.com/resource.tar.gz"
  end
end
"#;

    let formula = parse("someone", "tap", "example", source).unwrap();
    assert_eq!(formula.dependencies, vec!["openssl@3".to_string()]);
    assert_eq!(formula.build_dependencies, vec!["go".to_string()]);
    assert_stable_source(
        &formula,
        "https://example.com/example-1.0.0.tar.gz",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    );
}

#[test]
fn returns_unsupported_formula_when_neither_bottle_nor_source_is_available() {
    let source = r#"
class Terraform < Formula
  version "1.10.0"
end
"#;

    let err = parse("hashicorp", "tap", "terraform", source).unwrap_err();
    assert_matches!(err, Error::UnsupportedFormula { .. });
}
