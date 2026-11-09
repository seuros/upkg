//! The one reader of a manifest's text, whatever it came from.
//!
//! It walks toml's spanned document itself instead of handing it to serde:
//! serde's messages quote the values they reject (`invalid type: string
//! "..."`), and a manifest may come from a checkout nobody vetted. What it
//! says instead is where (line and column) and what, naming a key at most.
//! toml's own syntax messages are kept; they name grammar, never content.

use toml::de::{DeTable, DeValue};

/// The only key a manifest has.
const PACKAGES: &str = "packages";

/// The longest key name an error repeats.
const SHOWN_KEY: usize = 64;

/// The names `text` lists, in order, or why it isn't a manifest.
pub fn packages(text: &str) -> Result<Vec<String>, String> {
    let document = DeTable::parse(text)
        .map_err(|error| located(text, error.span(), error.message()))?
        .into_inner();
    // The table is sorted by name; report the unknown key written first.
    if let Some(key) = document
        .keys()
        .filter(|key| key.get_ref() != PACKAGES)
        .min_by_key(|key| key.span().start)
    {
        let what = format!(
            "unknown key {}; the only key is `{PACKAGES}`",
            shown(key.get_ref())
        );
        return Err(located(text, Some(key.span()), &what));
    }
    let Some(list) = document.values().next() else {
        return Err(format!("missing key `{PACKAGES}`"));
    };
    let DeValue::Array(items) = list.get_ref() else {
        let what = format!("`{PACKAGES}` must be a list of package names");
        return Err(located(text, Some(list.span()), &what));
    };
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let problem = match item.get_ref() {
                DeValue::String(name) => match check_name(name) {
                    Ok(()) => return Ok(name.to_string()),
                    Err(problem) => problem,
                },
                _ => "must be a string",
            };
            let what = format!("`{PACKAGES}[{index}]` {problem}");
            Err(located(text, Some(item.span()), &what))
        })
        .collect()
}

/// A name must reach the native manager as one plain argument: never empty,
/// never an option, never split by a shell.
pub fn check_name(name: &str) -> Result<(), &'static str> {
    if name.is_empty() {
        Err("is empty")
    } else if name.starts_with('-') {
        Err("starts with `-`; a package manager would read it as an option")
    } else if name.chars().any(|c| c.is_whitespace() || c.is_control()) {
        Err("contains whitespace or a control character")
    } else {
        Ok(())
    }
}

/// A key as an error may show it: a bare key in backticks, anything else
/// (quoted, long, escapes) unrepeated.
fn shown(key: &str) -> String {
    let bare = key.len() <= SHOWN_KEY
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if bare {
        format!("`{key}`")
    } else {
        "(a quoted key)".to_string()
    }
}

/// `what`, prefixed with the 1-based line and column `span` starts at.
fn located(text: &str, span: Option<std::ops::Range<usize>>, what: &str) -> String {
    let Some(before) = span.and_then(|span| text.get(..span.start)) else {
        return what.to_string();
    };
    let line = before.matches('\n').count() + 1;
    let column = before
        .rsplit('\n')
        .next()
        .unwrap_or_default()
        .chars()
        .count()
        + 1;
    format!("line {line}, column {column}: {what}")
}

#[cfg(test)]
mod tests;
