use std::path::PathBuf;

use crate::core::installer::install::create_installer;
use crate::core::network::api::ApiClient;
use crate::package_ref::{is_cask_name, normalize_app_name};
use crate::types::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageKindHint {
    Auto,
    App,
}

#[derive(Debug, Clone)]
pub struct InstallOptions {
    pub root: Option<PathBuf>,
    pub prefix: Option<PathBuf>,
    pub concurrency: usize,
    pub no_link: bool,
    pub build_from_source: bool,
    pub package_kind: PackageKindHint,
}

impl Default for InstallOptions {
    fn default() -> Self {
        Self {
            root: None,
            prefix: None,
            concurrency: 20,
            no_link: false,
            build_from_source: false,
            package_kind: PackageKindHint::Auto,
        }
    }
}

fn resolve_root_and_prefix(options: &InstallOptions) -> (PathBuf, PathBuf) {
    let default_prefix = default_prefix();
    let root = options.root.clone().unwrap_or_else(default_root);
    let prefix = options.prefix.clone().unwrap_or(default_prefix);
    (root, prefix)
}

fn build_runtime() -> Result<tokio::runtime::Runtime, Error> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| Error::ExecutionError {
            message: format!("failed to initialize runtime: {e}"),
        })
}

pub fn install(formulas: &[String], options: &InstallOptions) -> Result<(), Error> {
    if formulas.is_empty() {
        return Err(Error::InvalidArgument {
            message: "install requires at least one formula".to_string(),
        });
    }

    let (root, prefix) = resolve_root_and_prefix(options);

    crate::init::ensure_init(&root, &prefix, true)?;

    let runtime = build_runtime()?;

    runtime.block_on(async {
        let mut installer = create_installer(&root, &prefix, options.concurrency)?;

        crate::native_cli::commands::install::execute(
            &mut installer,
            formulas.to_vec(),
            options.no_link,
            options.build_from_source,
            options.package_kind,
        )
        .await
    })
}

pub fn uninstall(formulas: &[String], options: &InstallOptions) -> Result<(), Error> {
    if formulas.is_empty() {
        return Err(Error::InvalidArgument {
            message: "uninstall requires at least one formula".to_string(),
        });
    }

    let (root, prefix) = resolve_root_and_prefix(options);
    crate::init::ensure_init(&root, &prefix, true)?;
    let mut installer = create_installer(&root, &prefix, options.concurrency)?;

    for formula in package_requests(formulas, options.package_kind)? {
        installer.uninstall(&formula)?;
    }
    let _ = installer.gc()?;

    Ok(())
}

pub fn upgrade(formulas: &[String], options: &InstallOptions) -> Result<(), Error> {
    if formulas.is_empty() && options.package_kind == PackageKindHint::App {
        return Err(Error::InvalidArgument {
            message: "upgrade --app requires at least one app for now".to_string(),
        });
    }

    let (root, prefix) = resolve_root_and_prefix(options);
    crate::init::ensure_init(&root, &prefix, true)?;

    let runtime = build_runtime()?;

    runtime.block_on(async {
        let mut installer = create_installer(&root, &prefix, options.concurrency)?;
        let targets = if formulas.is_empty() {
            installed_formula_targets(installer.list_installed()?)
        } else {
            formulas.to_vec()
        };

        if targets.is_empty() {
            return Ok(());
        }

        crate::native_cli::commands::install::execute(
            &mut installer,
            targets,
            options.no_link,
            options.build_from_source,
            options.package_kind,
        )
        .await
    })
}

#[derive(Debug, Clone)]
pub struct SearchOptions {
    pub root: Option<PathBuf>,
    pub package_kind: PackageKindHint,
    pub exact: bool,
    pub refresh: bool,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            root: None,
            package_kind: PackageKindHint::Auto,
            exact: false,
            refresh: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchKind {
    Formula,
    Cask,
}

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub kind: SearchKind,
    pub name: String,
    pub version: String,
    pub desc: Option<String>,
}

pub fn search(query: &str, options: &SearchOptions) -> Result<Vec<SearchHit>, Error> {
    if query.trim().is_empty() {
        return Err(Error::InvalidArgument {
            message: "search query cannot be empty".to_string(),
        });
    }

    let root = options.root.clone().unwrap_or_else(default_root);
    let cache_dir = root.join("cache").join("homebrew-search");

    let runtime = build_runtime()?;
    let needle = query.to_ascii_lowercase();
    let exact = options.exact;
    let refresh = options.refresh;
    let kind = options.package_kind;

    runtime.block_on(async move {
        let client = ApiClient::new();
        let mut scored: Vec<(u8, SearchHit)> = Vec::new();

        if kind != PackageKindHint::App {
            let body = client.fetch_formula_index(&cache_dir, refresh).await?;
            scored.extend(score_formulae(&body, &needle, exact)?);
        }

        let body = client.fetch_cask_index(&cache_dir, refresh).await?;
        scored.extend(score_casks(&body, &needle, exact)?);

        scored.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| kind_rank(a.1.kind).cmp(&kind_rank(b.1.kind)))
                .then_with(|| a.1.name.cmp(&b.1.name))
        });

        Ok(scored.into_iter().map(|(_, hit)| hit).collect())
    })
}

fn kind_rank(kind: SearchKind) -> u8 {
    match kind {
        SearchKind::Formula => 0,
        SearchKind::Cask => 1,
    }
}

/// Lower rank = better match. Exact-mode hits are all rank 0 (no fuzziness).
/// Substring mode buckets:
///   0 = exact name/full_name match
///   1 = name/full_name starts with needle
///   2 = name/full_name contains needle
///   3 = alias or oldname contains needle
///   4 = description-only match
const RANK_EXACT: u8 = 0;
const RANK_PREFIX: u8 = 1;
const RANK_NAME_SUBSTR: u8 = 2;
const RANK_ALIAS_SUBSTR: u8 = 3;
const RANK_DESC_ONLY: u8 = 4;

fn score_formulae(body: &str, needle: &str, exact: bool) -> Result<Vec<(u8, SearchHit)>, Error> {
    let array: Vec<serde_json::Value> =
        serde_json::from_str(body).map_err(|e| Error::NetworkFailure {
            message: format!("failed to parse formula index: {e}"),
        })?;
    let mut hits = Vec::new();
    for entry in array.iter() {
        let name = entry
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let full_name = entry
            .get("full_name")
            .and_then(|v| v.as_str())
            .unwrap_or(name);
        let desc = entry.get("desc").and_then(|v| v.as_str()).unwrap_or("");
        let version = entry
            .get("versions")
            .and_then(|v| v.get("stable"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let aliases: Vec<&str> = entry
            .get("aliases")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|s| s.as_str()).collect())
            .unwrap_or_default();
        let oldnames: Vec<&str> = entry
            .get("oldnames")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|s| s.as_str()).collect())
            .unwrap_or_default();

        if let Some(rank) = score_hit(needle, exact, name, full_name, &aliases, &oldnames, desc) {
            hits.push((
                rank,
                SearchHit {
                    kind: SearchKind::Formula,
                    name: name.to_string(),
                    version,
                    desc: if desc.is_empty() {
                        None
                    } else {
                        Some(desc.to_string())
                    },
                },
            ));
        }
    }
    Ok(hits)
}

fn score_casks(body: &str, needle: &str, exact: bool) -> Result<Vec<(u8, SearchHit)>, Error> {
    let array: Vec<serde_json::Value> =
        serde_json::from_str(body).map_err(|e| Error::NetworkFailure {
            message: format!("failed to parse cask index: {e}"),
        })?;
    let mut hits = Vec::new();
    for entry in array.iter() {
        let token = entry
            .get("token")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if token.is_empty() {
            continue;
        }
        let desc = entry.get("desc").and_then(|v| v.as_str()).unwrap_or("");
        let version = entry
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let names: Vec<String> = entry
            .get("name")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|s| s.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let name_refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();

        if let Some(rank) = score_hit(needle, exact, token, token, &name_refs, &[], desc) {
            hits.push((
                rank,
                SearchHit {
                    kind: SearchKind::Cask,
                    name: token.to_string(),
                    version,
                    desc: if desc.is_empty() {
                        None
                    } else {
                        Some(desc.to_string())
                    },
                },
            ));
        }
    }
    Ok(hits)
}

fn score_hit(
    needle: &str,
    exact: bool,
    name: &str,
    full_name: &str,
    aliases: &[&str],
    oldnames: &[&str],
    desc: &str,
) -> Option<u8> {
    if exact {
        let hit = name.eq_ignore_ascii_case(needle)
            || full_name.eq_ignore_ascii_case(needle)
            || aliases.iter().any(|a| a.eq_ignore_ascii_case(needle))
            || oldnames.iter().any(|a| a.eq_ignore_ascii_case(needle));
        return if hit { Some(RANK_EXACT) } else { None };
    }

    let needle_lc = needle.to_ascii_lowercase();
    let name_lc = name.to_ascii_lowercase();
    let full_lc = full_name.to_ascii_lowercase();

    if name_lc == needle_lc || full_lc == needle_lc {
        return Some(RANK_EXACT);
    }
    if name_lc.starts_with(&needle_lc) || full_lc.starts_with(&needle_lc) {
        return Some(RANK_PREFIX);
    }
    if name_lc.contains(&needle_lc) || full_lc.contains(&needle_lc) {
        return Some(RANK_NAME_SUBSTR);
    }
    if aliases
        .iter()
        .any(|h| h.to_ascii_lowercase().contains(&needle_lc))
        || oldnames
            .iter()
            .any(|h| h.to_ascii_lowercase().contains(&needle_lc))
    {
        return Some(RANK_ALIAS_SUBSTR);
    }
    if desc.to_ascii_lowercase().contains(&needle_lc) {
        return Some(RANK_DESC_ONLY);
    }
    None
}

pub fn list(
    options: &InstallOptions,
) -> Result<Vec<crate::core::storage::receipt::InstalledKeg>, Error> {
    let (root, prefix) = resolve_root_and_prefix(options);
    crate::init::ensure_init(&root, &prefix, true)?;
    let installer = create_installer(&root, &prefix, options.concurrency)?;
    installer.list_installed()
}

fn installed_formula_targets(
    installed: Vec<crate::core::storage::receipt::InstalledKeg>,
) -> Vec<String> {
    let mut targets: Vec<String> = installed
        .into_iter()
        .map(|keg| keg.name)
        .filter(|name| !is_cask_name(name))
        .collect();
    targets.sort();
    targets.dedup();
    targets
}

fn default_prefix() -> PathBuf {
    env_path("UPKG_PREFIX").unwrap_or_else(|| {
        if cfg!(target_arch = "aarch64") {
            PathBuf::from("/opt/homebrew")
        } else {
            PathBuf::from("/usr/local")
        }
    })
}

fn default_root() -> PathBuf {
    env_path("UPKG_ROOT").unwrap_or_else(default_prefix)
}

fn env_path(name: &str) -> Option<PathBuf> {
    env_path_value(std::env::var_os(name))
}

fn env_path_value(value: Option<std::ffi::OsString>) -> Option<PathBuf> {
    value
        .filter(|value| !value.as_os_str().is_empty())
        .map(PathBuf::from)
}

fn package_requests(
    packages: &[String],
    kind: PackageKindHint,
) -> Result<Vec<String>, crate::types::Error> {
    match kind {
        PackageKindHint::Auto => Ok(packages.to_vec()),
        PackageKindHint::App => packages
            .iter()
            .map(|package| normalize_app_name(package))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::storage::receipt::InstalledKeg;

    #[test]
    fn score_hit_substring_in_name() {
        assert_eq!(
            score_hit("rip", false, "ripgrep", "ripgrep", &[], &[], "fast grep"),
            Some(RANK_PREFIX),
        );
    }

    #[test]
    fn score_hit_case_insensitive() {
        assert_eq!(
            score_hit("RIPGREP", false, "ripgrep", "ripgrep", &[], &[], ""),
            Some(RANK_EXACT),
        );
    }

    #[test]
    fn score_hit_substring_in_desc_only() {
        assert_eq!(
            score_hit(
                "fast",
                false,
                "ripgrep",
                "ripgrep",
                &[],
                &[],
                "a faster grep"
            ),
            Some(RANK_DESC_ONLY),
        );
    }

    #[test]
    fn score_hit_alias_substring() {
        assert_eq!(
            score_hit("rg", false, "ripgrep", "ripgrep", &["rg"], &[], ""),
            Some(RANK_ALIAS_SUBSTR),
        );
    }

    #[test]
    fn score_hit_prefix_beats_substring() {
        let a = score_hit("grep", false, "grep-foo", "grep-foo", &[], &[], "");
        let b = score_hit("grep", false, "ripgrep", "ripgrep", &[], &[], "");
        assert_eq!(a, Some(RANK_PREFIX));
        assert_eq!(b, Some(RANK_NAME_SUBSTR));
        assert!(a.unwrap() < b.unwrap());
    }

    #[test]
    fn score_hit_no_match_returns_none() {
        assert_eq!(
            score_hit("xyz", false, "ripgrep", "ripgrep", &[], &[], ""),
            None,
        );
    }

    #[test]
    fn score_hit_exact_mode_requires_full_name() {
        assert_eq!(
            score_hit("git", true, "git", "git", &[], &[], ""),
            Some(RANK_EXACT)
        );
        assert_eq!(score_hit("gi", true, "git", "git", &[], &[], ""), None);
    }

    #[test]
    fn score_hit_exact_mode_accepts_alias() {
        assert_eq!(
            score_hit("rg", true, "ripgrep", "ripgrep", &["rg"], &[], ""),
            Some(RANK_EXACT),
        );
    }

    #[test]
    fn score_hit_exact_mode_ignores_desc() {
        assert_eq!(
            score_hit(
                "fast",
                true,
                "ripgrep",
                "ripgrep",
                &[],
                &[],
                "fast grep tool"
            ),
            None,
        );
    }

    #[test]
    fn score_formulae_skips_entries_without_name() {
        let body = r#"[{"versions":{"stable":"1.0"}}]"#;
        let hits = score_formulae(body, "anything", false).unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn score_formulae_extracts_metadata() {
        let body = r#"[{
            "name": "ripgrep",
            "full_name": "ripgrep",
            "desc": "Search tool",
            "versions": {"stable": "14.1.1"},
            "aliases": ["rg"]
        }]"#;
        let hits = score_formulae(body, "rip", false).unwrap();
        assert_eq!(hits.len(), 1);
        let (rank, hit) = &hits[0];
        assert_eq!(*rank, RANK_PREFIX);
        assert_eq!(hit.name, "ripgrep");
        assert_eq!(hit.version, "14.1.1");
        assert_eq!(hit.kind, SearchKind::Formula);
    }

    #[test]
    fn score_casks_extracts_metadata() {
        let body = r#"[{
            "token": "ghostty",
            "name": ["Ghostty"],
            "desc": "Terminal emulator",
            "version": "1.3.0"
        }]"#;
        let hits = score_casks(body, "ghost", false).unwrap();
        assert_eq!(hits.len(), 1);
        let (rank, hit) = &hits[0];
        assert_eq!(*rank, RANK_PREFIX);
        assert_eq!(hit.name, "ghostty");
        assert_eq!(hit.version, "1.3.0");
        assert_eq!(hit.kind, SearchKind::Cask);
    }

    #[test]
    fn score_casks_matches_human_name() {
        let body = r#"[{
            "token": "visual-studio-code",
            "name": ["Visual Studio Code"],
            "desc": "Code editor",
            "version": "1.0"
        }]"#;
        let hits = score_casks(body, "studio code", false).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, RANK_ALIAS_SUBSTR);
    }

    #[test]
    fn no_arg_upgrade_targets_exclude_app_casks() {
        let targets = installed_formula_targets(vec![
            InstalledKeg {
                name: "ripgrep".to_string(),
                version: "14.1.1".to_string(),
                store_key: "rg-sha".to_string(),
            },
            InstalledKeg {
                name: "cask:ghostty".to_string(),
                version: "1.3.0".to_string(),
                store_key: String::new(),
            },
            InstalledKeg {
                name: "ripgrep".to_string(),
                version: "14.1.1".to_string(),
                store_key: "rg-sha".to_string(),
            },
        ]);

        assert_eq!(targets, vec!["ripgrep".to_string()]);
    }

    #[test]
    fn env_path_value_ignores_missing_and_empty_values() {
        assert_eq!(env_path_value(None), None);
        assert_eq!(env_path_value(Some(std::ffi::OsString::new())), None);
    }

    #[test]
    fn env_path_value_accepts_non_empty_path() {
        assert_eq!(
            env_path_value(Some(std::ffi::OsString::from("/tmp/upkg-test"))),
            Some(PathBuf::from("/tmp/upkg-test"))
        );
    }
}
