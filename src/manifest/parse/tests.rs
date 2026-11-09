use super::*;

#[test]
fn located_counts_lines_and_characters_from_one() {
    let text = "a\nßb = 1\n";
    // `b` is byte 4 (ß is two bytes) but the second character of line 2.
    assert_eq!(located(text, Some(4..5), "here"), "line 2, column 2: here");
    assert_eq!(located(text, Some(0..1), "here"), "line 1, column 1: here");
}

#[test]
fn located_without_a_usable_span_says_only_what() {
    assert_eq!(located("abc", None, "here"), "here");
    assert_eq!(located("abc", Some(10..11), "here"), "here");
}

#[test]
fn only_bare_keys_are_repeated() {
    assert_eq!(shown("pakages"), "`pakages`");
    assert_eq!(shown("per-os_list2"), "`per-os_list2`");
    assert_eq!(shown("two words"), "(a quoted key)");
    assert_eq!(shown("\u{1b}[31m"), "(a quoted key)");
    assert_eq!(shown(&"k".repeat(SHOWN_KEY + 1)), "(a quoted key)");
}

#[test]
fn every_problem_with_a_name_is_named() {
    assert_eq!(check_name("git"), Ok(()));
    assert_eq!(check_name(""), Err("is empty"));
    assert!(check_name("-y").unwrap_err().starts_with("starts with `-`"));
    assert!(check_name("a\tb").unwrap_err().contains("whitespace"));
    assert!(check_name("a\u{7}").unwrap_err().contains("control"));
}

#[test]
fn literal_and_multiline_arrays_are_lists_like_any_other() {
    let text = "packages = [\n  'git',  # a comment\n  \"curl\",\n]\n";
    assert_eq!(packages(text).unwrap(), ["git", "curl"]);
}
