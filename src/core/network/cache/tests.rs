use super::*;

#[test]
fn stores_and_retrieves_cache_entry() {
    let cache = ApiCache::in_memory().unwrap();

    let entry = CacheEntry {
        etag: Some("abc123".to_string()),
        last_modified: None,
        body: r#"{"name":"foo"}"#.to_string(),
    };

    cache.put("https://example.com/foo.json", &entry).unwrap();
    let retrieved = cache.get("https://example.com/foo.json").unwrap();

    assert_eq!(retrieved.etag, Some("abc123".to_string()));
    assert_eq!(retrieved.body, r#"{"name":"foo"}"#);
}

#[test]
fn returns_none_for_missing_entry() {
    let cache = ApiCache::in_memory().unwrap();
    assert!(cache.get("https://example.com/nonexistent.json").is_none());
}
