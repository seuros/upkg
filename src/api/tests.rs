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
