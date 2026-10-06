use crate::core::build::formula_parser::{parse_ruby, parse_string};
use crate::types::formula::{
    Bottle, BottleFile, BottleStable, FormulaUrls, KegOnly, SourceUrl, Versions,
};
use crate::types::{Error, Formula};
use std::collections::BTreeMap;
use tree_sitter::{Node, Tree};

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
        Ok(Self {
            tree: parse_ruby(source)?,
            source,
        })
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
#[path = "tap_formula/tests.rs"]
mod tests;
