use crate::types::formula::{
    Bottle, BottleFile, BottleStable, FormulaUrls, KegOnly, SourceUrl, Versions,
};
use crate::types::{Error, Formula};
use std::collections::BTreeMap;
use tree_sitter::{Node, Parser, Tree};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TapFormulaRef {
    pub owner: String,
    pub repo: String,
    pub formula: String,
}

pub fn parse_tap_formula_ref(input: &str) -> Option<TapFormulaRef> {
    let mut parts = input.split('/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    let formula = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    if owner.is_empty() || repo.is_empty() || formula.is_empty() {
        return None;
    }
    Some(TapFormulaRef {
        owner: owner.to_string(),
        repo: repo.to_string(),
        formula: formula.to_string(),
    })
}

pub fn parse_tap_formula_ruby(spec: &TapFormulaRef, source: &str) -> Result<Formula, Error> {
    let parsed = ParsedTapFormula::parse(source)?;
    let formula_body = parsed.formula_body();

    let stable = parse_version(&parsed, formula_body).unwrap_or_else(|| "0".to_string());
    let revision = parse_revision(&parsed, formula_body).unwrap_or(0);
    let dependencies = parse_runtime_dependencies(&parsed, formula_body);
    let build_dependencies = parse_build_dependencies(&parsed, formula_body);
    let parsed_source_url = parse_source_url(&parsed, formula_body);
    let bottle = parse_bottle(spec, &parsed, formula_body, &stable, revision);

    let source_url = match parsed_source_url {
        ParsedSourceUrl::PresentWithChecksum(source_url) => Some(source_url),
        ParsedSourceUrl::PresentMissingChecksum => {
            if bottle.is_none() {
                return Err(Error::UnsupportedFormula {
                    name: spec.formula.clone(),
                    reason: "tap formula source url is missing sha256".to_string(),
                });
            }
            None
        }
        ParsedSourceUrl::NotPresent => None,
    };

    if bottle.is_none() && source_url.is_none() {
        return Err(Error::UnsupportedFormula {
            name: spec.formula.clone(),
            reason: "tap formula does not provide bottle data or source url".to_string(),
        });
    }

    Ok(Formula {
        name: spec.formula.clone(),
        versions: Versions { stable },
        dependencies,
        bottle: bottle.unwrap_or_else(empty_bottle),
        revision,
        keg_only: KegOnly::default(),
        build_dependencies,
        urls: source_url.map(|stable| FormulaUrls {
            stable: Some(stable),
            head: None,
        }),
        ruby_source_path: None,
        ruby_source_checksum: None,
        uses_from_macos: Vec::new(),
        requirements: Vec::new(),
        variations: None,
    })
}

#[derive(Debug)]
struct ParsedTapFormula<'a> {
    tree: Tree,
    source: &'a str,
}

impl<'a> ParsedTapFormula<'a> {
    fn parse(source: &'a str) -> Result<Self, Error> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_ruby::LANGUAGE.into())
            .map_err(|e| Error::ExecutionError {
                message: format!("failed to load Ruby grammar: {e}"),
            })?;

        let tree = parser
            .parse(source, None)
            .ok_or_else(|| Error::ExecutionError {
                message: "failed to parse Ruby formula".to_string(),
            })?;

        Ok(Self { tree, source })
    }

    fn source_bytes(&self) -> &'a [u8] {
        self.source.as_bytes()
    }

    fn formula_body(&self) -> Option<Node<'_>> {
        find_formula_class_body(self.tree.root_node(), self.source_bytes())
    }
}

fn parse_version(parsed: &ParsedTapFormula<'_>, body: Option<Node<'_>>) -> Option<String> {
    let body = body?;
    let version = find_top_level_call(body, parsed.source_bytes(), "version")
        .and_then(|call| first_string_argument(call, parsed.source_bytes()));
    if version.is_some() {
        return version;
    }

    find_top_level_call(body, parsed.source_bytes(), "url")
        .and_then(|call| first_string_argument(call, parsed.source_bytes()))
        .and_then(|url| infer_version_from_url(&url))
}

fn infer_version_from_url(url: &str) -> Option<String> {
    for marker in ["refs/tags/", "archive/", "download/"] {
        let Some(index) = url.find(marker) else {
            continue;
        };
        let mut raw = &url[index + marker.len()..];
        if raw.starts_with('v') && raw.as_bytes().get(1).is_some_and(u8::is_ascii_digit) {
            raw = &raw[1..];
        }
        if !raw.as_bytes().first().is_some_and(u8::is_ascii_digit) {
            continue;
        }

        let end = raw
            .find(|c: char| !c.is_ascii_alphanumeric() && !matches!(c, '.' | '_' | '+' | '-'))
            .unwrap_or(raw.len());
        return Some(normalize_inferred_version(&raw[..end]));
    }

    None
}

fn normalize_inferred_version(raw: &str) -> String {
    let mut v = raw.to_string();
    for suffix in [".tar.gz", ".tar.xz", ".tar.bz2", ".tgz", ".zip"] {
        if v.ends_with(suffix) {
            v.truncate(v.len() - suffix.len());
            break;
        }
    }
    v
}

fn parse_revision(parsed: &ParsedTapFormula<'_>, body: Option<Node<'_>>) -> Option<u32> {
    find_top_level_call(body?, parsed.source_bytes(), "revision")
        .and_then(|call| first_integer_argument(call, parsed.source_bytes()))
}

fn parse_runtime_dependencies(
    parsed: &ParsedTapFormula<'_>,
    body: Option<Node<'_>>,
) -> Vec<String> {
    let mut deps = Vec::new();

    for call in top_level_calls(body, parsed.source_bytes()) {
        if call_method(call, parsed.source_bytes()) != Some("depends_on") {
            continue;
        }
        let Some(dep) = dependency_name(call, parsed.source_bytes()) else {
            continue;
        };
        let tags = dependency_tags(call, parsed.source_bytes());
        if !tags.iter().any(|tag| tag == "build" || tag == "test") {
            deps.push(dep);
        }
    }

    deps.sort_unstable();
    deps.dedup();
    deps
}

fn parse_build_dependencies(parsed: &ParsedTapFormula<'_>, body: Option<Node<'_>>) -> Vec<String> {
    let mut deps = Vec::new();

    for call in top_level_calls(body, parsed.source_bytes()) {
        if call_method(call, parsed.source_bytes()) != Some("depends_on") {
            continue;
        }
        let Some(dep) = dependency_name(call, parsed.source_bytes()) else {
            continue;
        };
        if dependency_tags(call, parsed.source_bytes())
            .iter()
            .any(|tag| tag == "build")
        {
            deps.push(dep);
        }
    }

    deps.sort_unstable();
    deps.dedup();
    deps
}

enum ParsedSourceUrl {
    NotPresent,
    PresentMissingChecksum,
    PresentWithChecksum(SourceUrl),
}

fn parse_source_url(parsed: &ParsedTapFormula<'_>, body: Option<Node<'_>>) -> ParsedSourceUrl {
    let mut url: Option<String> = None;
    let mut checksum: Option<String> = None;

    for call in top_level_calls(body, parsed.source_bytes()) {
        match call_method(call, parsed.source_bytes()) {
            Some("url") if url.is_none() => {
                url = first_string_argument(call, parsed.source_bytes());
            }
            Some("sha256") if checksum.is_none() => {
                checksum = first_string_argument(call, parsed.source_bytes())
                    .filter(|sha| is_sha256_hex(sha));
            }
            _ => {}
        }

        if url.is_some() && checksum.is_some() {
            break;
        }
    }

    match (url, checksum) {
        (Some(url), Some(checksum)) => ParsedSourceUrl::PresentWithChecksum(SourceUrl {
            url,
            checksum: Some(checksum),
            tag: None,
            revision: None,
        }),
        (Some(_), None) => ParsedSourceUrl::PresentMissingChecksum,
        _ => ParsedSourceUrl::NotPresent,
    }
}

fn find_formula_class_body<'a>(root: Node<'a>, source: &'a [u8]) -> Option<Node<'a>> {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == "class" && class_extends_formula(node, source) {
            return class_body(node);
        }

        let mut cursor = node.walk();
        let children: Vec<_> = node.named_children(&mut cursor).collect();
        stack.extend(children.into_iter().rev());
    }

    None
}

fn class_extends_formula(node: Node<'_>, source: &[u8]) -> bool {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).any(|child| {
        if child.kind() != "superclass" {
            return false;
        }

        let mut cursor = child.walk();
        child
            .named_children(&mut cursor)
            .any(|grandchild| grandchild.utf8_text(source).ok() == Some("Formula"))
    })
}

fn class_body<'a>(node: Node<'a>) -> Option<Node<'a>> {
    if let Some(body) = node.child_by_field_name("body") {
        return Some(body);
    }

    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|child| child.kind() == "body_statement")
}

fn top_level_calls<'a>(body: Option<Node<'a>>, source: &'a [u8]) -> Vec<Node<'a>> {
    let Some(body) = body else {
        return Vec::new();
    };

    let mut calls = Vec::new();
    let mut cursor = body.walk();
    for child in body.named_children(&mut cursor) {
        if child.kind() == "call" && call_method(child, source).is_some() {
            calls.push(child);
        }
    }
    calls
}

fn find_top_level_call<'a>(body: Node<'a>, source: &'a [u8], method: &str) -> Option<Node<'a>> {
    top_level_calls(Some(body), source)
        .into_iter()
        .find(|call| call_method(*call, source) == Some(method))
}

fn descendant_calls<'a>(node: Node<'a>, source: &'a [u8]) -> Vec<Node<'a>> {
    let mut calls = Vec::new();
    let mut stack = Vec::new();
    let mut cursor = node.walk();
    stack.extend(node.named_children(&mut cursor));

    while let Some(current) = stack.pop() {
        if current.kind() == "call" && call_method(current, source).is_some() {
            calls.push(current);
        }

        let mut cursor = current.walk();
        stack.extend(current.named_children(&mut cursor));
    }

    calls.sort_by_key(Node::start_byte);
    calls
}

fn find_descendant_call<'a>(node: Node<'a>, source: &'a [u8], method: &str) -> Option<Node<'a>> {
    descendant_calls(node, source)
        .into_iter()
        .find(|call| call_method(*call, source) == Some(method))
}

fn call_method<'a>(node: Node<'a>, source: &'a [u8]) -> Option<&'a str> {
    node.child_by_field_name("method")?.utf8_text(source).ok()
}

fn call_arguments<'a>(node: Node<'a>) -> Option<Node<'a>> {
    node.child_by_field_name("arguments")
}

fn first_string_argument(node: Node<'_>, source: &[u8]) -> Option<String> {
    let arguments = call_arguments(node)?;
    let mut cursor = arguments.walk();
    for child in arguments.named_children(&mut cursor) {
        match child.kind() {
            "string" => return parse_string(child, source),
            "pair" => {
                let key = child.child_by_field_name("key")?;
                if key.kind() == "string" {
                    return parse_string(key, source);
                }
            }
            _ => {}
        }
    }
    None
}

fn first_integer_argument(node: Node<'_>, source: &[u8]) -> Option<u32> {
    let arguments = call_arguments(node)?;
    let mut cursor = arguments.walk();
    for child in arguments.named_children(&mut cursor) {
        if child.kind() == "integer" {
            return child.utf8_text(source).ok()?.parse::<u32>().ok();
        }
    }
    None
}

fn dependency_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    first_string_argument(node, source)
}

fn dependency_tags(node: Node<'_>, source: &[u8]) -> Vec<String> {
    let Some(arguments) = call_arguments(node) else {
        return Vec::new();
    };

    symbol_values(arguments, source)
}

fn sha256_pairs(node: Node<'_>, source: &[u8]) -> Vec<(String, String)> {
    let Some(arguments) = call_arguments(node) else {
        return Vec::new();
    };

    let mut pairs = Vec::new();
    let mut cursor = arguments.walk();
    for child in arguments.named_children(&mut cursor) {
        if child.kind() != "pair" {
            continue;
        }
        let Some(key) = child.child_by_field_name("key") else {
            continue;
        };
        let Some(value) = child.child_by_field_name("value") else {
            continue;
        };
        let Some(tag) = symbol_key(key, source) else {
            continue;
        };
        if !tag
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            continue;
        }
        let Some(sha) = parse_string(value, source).filter(|sha| is_sha256_hex(sha)) else {
            continue;
        };
        pairs.push((tag, sha));
    }

    pairs
}

fn symbol_values(node: Node<'_>, source: &[u8]) -> Vec<String> {
    let mut values = Vec::new();
    let mut stack = vec![node];

    while let Some(current) = stack.pop() {
        if let Some(value) = symbol_value(current, source) {
            values.push(value);
            continue;
        }

        let mut cursor = current.walk();
        stack.extend(current.named_children(&mut cursor));
    }

    values
}

fn symbol_key(node: Node<'_>, source: &[u8]) -> Option<String> {
    match node.kind() {
        "hash_key_symbol" => node.utf8_text(source).ok().map(ToString::to_string),
        "simple_symbol" | "symbol" => symbol_value(node, source),
        _ => None,
    }
}

fn symbol_value(node: Node<'_>, source: &[u8]) -> Option<String> {
    match node.kind() {
        "simple_symbol" | "symbol" => node
            .utf8_text(source)
            .ok()
            .map(|value| value.trim_start_matches(':').to_string()),
        _ => None,
    }
}

fn parse_string(node: Node<'_>, source: &[u8]) -> Option<String> {
    if node.kind() != "string" {
        return None;
    }

    if let Some(content) = node.child_by_field_name("content")
        && let Ok(value) = content.utf8_text(source)
    {
        return Some(value.to_string());
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "string_content" {
            return child.utf8_text(source).ok().map(ToString::to_string);
        }
    }

    Some(String::new())
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn parse_bottle(
    spec: &TapFormulaRef,
    parsed: &ParsedTapFormula<'_>,
    body: Option<Node<'_>>,
    stable: &str,
    revision: u32,
) -> Option<Bottle> {
    let bottle_call = find_top_level_call(body?, parsed.source_bytes(), "bottle")?;

    let root_url = parse_root_url(bottle_call, parsed.source_bytes())
        .unwrap_or_else(|| format!("https://ghcr.io/v2/{}/{}", spec.owner, spec.repo));
    let rebuild = parse_rebuild(bottle_call, parsed.source_bytes()).unwrap_or(0);
    let files = parse_bottle_files(
        spec,
        &root_url,
        stable,
        revision,
        rebuild,
        bottle_call,
        parsed.source_bytes(),
    );

    if files.is_empty() {
        return None;
    }

    Some(Bottle {
        stable: BottleStable { files, rebuild },
    })
}

fn empty_bottle() -> Bottle {
    Bottle {
        stable: BottleStable {
            files: BTreeMap::new(),
            rebuild: 0,
        },
    }
}

fn parse_root_url(bottle_call: Node<'_>, source: &[u8]) -> Option<String> {
    find_descendant_call(bottle_call, source, "root_url")
        .and_then(|call| first_string_argument(call, source))
}

fn parse_rebuild(bottle_call: Node<'_>, source: &[u8]) -> Option<u32> {
    find_descendant_call(bottle_call, source, "rebuild")
        .and_then(|call| first_integer_argument(call, source))
}

fn parse_bottle_files(
    spec: &TapFormulaRef,
    root_url: &str,
    stable: &str,
    revision: u32,
    rebuild: u32,
    bottle_call: Node<'_>,
    source: &[u8],
) -> BTreeMap<String, BottleFile> {
    let mut files = BTreeMap::new();

    for call in descendant_calls(bottle_call, source) {
        if call_method(call, source) != Some("sha256") {
            continue;
        }

        for (tag, sha) in sha256_pairs(call, source) {
            if tag == "cellar" {
                continue;
            }
            let url = build_bottle_url(spec, root_url, stable, revision, rebuild, &tag, &sha);
            files.insert(tag, BottleFile { url, sha256: sha });
        }
    }

    files
}

fn build_bottle_url(
    spec: &TapFormulaRef,
    root_url: &str,
    stable: &str,
    revision: u32,
    rebuild: u32,
    tag: &str,
    sha: &str,
) -> String {
    let normalized = root_url.trim_end_matches('/');
    if normalized.contains("/v2/") {
        return format!("{}/{}/blobs/sha256:{}", normalized, spec.formula, sha);
    }

    let effective_version = if revision > 0 {
        format!("{stable}_{revision}")
    } else {
        stable.to_string()
    };

    if rebuild > 0 {
        format!(
            "{}/{}-{}.{}.{}.bottle.tar.gz",
            normalized, spec.formula, effective_version, rebuild, tag
        )
    } else {
        format!(
            "{}/{}-{}.{}.bottle.tar.gz",
            normalized, spec.formula, effective_version, tag
        )
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn parses_tap_formula_reference() {
        let parsed = parse_tap_formula_ref("hashicorp/tap/terraform").unwrap();
        assert_eq!(parsed.owner, "hashicorp");
        assert_eq!(parsed.repo, "tap");
        assert_eq!(parsed.formula, "terraform");
    }

    #[test]
    fn rejects_non_tap_reference() {
        assert!(parse_tap_formula_ref("jq").is_none());
        assert!(parse_tap_formula_ref("a/b").is_none());
        assert!(parse_tap_formula_ref("a/b/c/d").is_none());
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

        let spec = TapFormulaRef {
            owner: "hashicorp".to_string(),
            repo: "tap".to_string(),
            formula: "terraform".to_string(),
        };

        let formula = parse_tap_formula_ruby(&spec, source).unwrap();
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

        let spec = TapFormulaRef {
            owner: "hashicorp".to_string(),
            repo: "tap".to_string(),
            formula: "terraform".to_string(),
        };

        let formula = parse_tap_formula_ruby(&spec, source).unwrap();
        let url = &formula.bottle.stable.files["arm64_sonoma"].url;
        assert_eq!(
            url,
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
        let spec = TapFormulaRef {
            owner: "messense".to_string(),
            repo: "tap".to_string(),
            formula: "ttfb".to_string(),
        };
        let formula = parse_tap_formula_ruby(&spec, source).unwrap();
        let url = &formula.bottle.stable.files["x86_64_linux"].url;
        assert_eq!(
            url,
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
        let spec = TapFormulaRef {
            owner: "simnalamburt".to_string(),
            repo: "x".to_string(),
            formula: "jaso".to_string(),
        };
        let formula = parse_tap_formula_ruby(&spec, source).unwrap();
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

        let spec = TapFormulaRef {
            owner: "hashicorp".to_string(),
            repo: "tap".to_string(),
            formula: "terraform".to_string(),
        };
        let formula = parse_tap_formula_ruby(&spec, source).unwrap();

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

        let spec = TapFormulaRef {
            owner: "jandedobbeleer".to_string(),
            repo: "oh-my-posh".to_string(),
            formula: "oh-my-posh".to_string(),
        };

        let formula = parse_tap_formula_ruby(&spec, source).unwrap();
        assert!(formula.bottle.stable.files.is_empty());
        assert_eq!(formula.build_dependencies, vec!["go@1.26".to_string()]);

        let stable = formula
            .urls
            .as_ref()
            .and_then(|u| u.stable.as_ref())
            .expect("stable source url should be parsed");
        assert_eq!(
            stable.url,
            "https://github.com/JanDeDobbeleer/oh-my-posh/archive/v29.3.0.tar.gz"
        );
        assert_eq!(
            stable.checksum.as_deref(),
            Some("ff39f6ef2b4ca2d7d766f2802520b023986a5d6dbcd59fba685a9e5bacf41993")
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

        let spec = TapFormulaRef {
            owner: "someone".to_string(),
            repo: "tap".to_string(),
            formula: "example".to_string(),
        };

        let formula = parse_tap_formula_ruby(&spec, source).unwrap();
        let stable = formula
            .urls
            .as_ref()
            .and_then(|u| u.stable.as_ref())
            .expect("stable source url should be parsed");

        assert_eq!(stable.url, "https://example.com/example-1.0.0.tar.gz");
        assert_eq!(
            stable.checksum.as_deref(),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
    }

    #[test]
    fn source_url_without_sha256_is_unsupported() {
        let source = r#"
class Example < Formula
  url "https://example.com/example-1.0.0.tar.gz"
end
"#;

        let spec = TapFormulaRef {
            owner: "someone".to_string(),
            repo: "tap".to_string(),
            formula: "example".to_string(),
        };

        let err = parse_tap_formula_ruby(&spec, source).unwrap_err();
        assert!(matches!(
            err,
            Error::UnsupportedFormula { reason, .. }
            if reason.contains("missing sha256")
        ));
    }

    #[test]
    fn source_url_without_top_level_sha256_is_unsupported_even_if_nested_has_sha256() {
        let source = r#"
class Example < Formula
  url "https://example.com/example-1.0.0.tar.gz"

  resource "extra" do
    url "https://example.com/resource.tar.gz"
    sha256 "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
  end
end
"#;

        let spec = TapFormulaRef {
            owner: "someone".to_string(),
            repo: "tap".to_string(),
            formula: "example".to_string(),
        };

        let err = parse_tap_formula_ruby(&spec, source).unwrap_err();
        assert!(matches!(
            err,
            Error::UnsupportedFormula { reason, .. }
            if reason.contains("missing sha256")
        ));
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

        let spec = TapFormulaRef {
            owner: "someone".to_string(),
            repo: "tap".to_string(),
            formula: "example".to_string(),
        };

        let formula = parse_tap_formula_ruby(&spec, source).unwrap();
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

        let spec = TapFormulaRef {
            owner: "someone".to_string(),
            repo: "tap".to_string(),
            formula: "example".to_string(),
        };

        let formula = parse_tap_formula_ruby(&spec, source).unwrap();
        assert_eq!(formula.dependencies, vec!["openssl@3".to_string()]);
        assert_eq!(formula.build_dependencies, vec!["go".to_string()]);

        let stable = formula
            .urls
            .as_ref()
            .and_then(|u| u.stable.as_ref())
            .expect("stable source url should be parsed");
        assert_eq!(stable.url, "https://example.com/example-1.0.0.tar.gz");
        assert_eq!(
            stable.checksum.as_deref(),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
    }

    #[test]
    fn returns_unsupported_formula_when_neither_bottle_nor_source_is_available() {
        let source = r#"
class Terraform < Formula
  version "1.10.0"
end
"#;

        let spec = TapFormulaRef {
            owner: "hashicorp".to_string(),
            repo: "tap".to_string(),
            formula: "terraform".to_string(),
        };

        let err = parse_tap_formula_ruby(&spec, source).unwrap_err();
        assert!(matches!(err, Error::UnsupportedFormula { .. }));
    }
}
