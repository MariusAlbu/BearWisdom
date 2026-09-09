use super::*;

#[test]
fn normalize_path_key_folds_dot_segments() {
    use std::path::Path;
    // The same file reached via an unfolded `..`/`.` route must canonicalize to
    // one key, so the cache doesn't accumulate duplicate rows per route.
    let canon = normalize_path_key(Path::new("/proj/node_modules/dep/dist/x.d.ts"));
    let unfolded = normalize_path_key(Path::new("/proj/node_modules/dep/types/../dist/./x.d.ts"));
    assert_eq!(canon, unfolded);
    assert_eq!(canon, "/proj/node_modules/dep/dist/x.d.ts");
}

#[cfg(windows)]
#[test]
fn normalize_path_key_unifies_separators() {
    use std::path::Path;
    let bs = normalize_path_key(Path::new(r"F:\Work\node_modules\dep\x.d.ts"));
    let fwd = normalize_path_key(Path::new("F:/Work/node_modules/dep/x.d.ts"));
    assert_eq!(bs, fwd);
    assert_eq!(bs, "F:/Work/node_modules/dep/x.d.ts");
    assert_eq!(
        cache_key(Path::new(r"F:\a\b"), "h"),
        cache_key(Path::new("F:/a/b"), "h"),
        "cache_key must be separator-independent"
    );
}

#[test]
fn content_hash_is_stable_and_distinct() {
    assert_eq!(content_hash(b"hello"), content_hash(b"hello"));
    assert_ne!(content_hash(b"hello"), content_hash(b"world"));
    // sha-256 hex is 64 chars.
    assert_eq!(content_hash(b"x").len(), 64);
}

#[test]
fn cache_key_embeds_schema_version() {
    use std::path::Path;
    let key = cache_key(Path::new("/a/b.d.ts"), "deadbeef");
    assert!(
        key.starts_with(&format!("{EXTRACTOR_SCHEMA_VERSION}:")),
        "schema version must prefix the key so a bump flushes all prior entries"
    );
}

#[test]
fn pre_scoped_owner_payloads_cannot_match_current_cache_keys() {
    let current = cache_key(Path::new("/source/lib.rs"), "unchanged");
    assert!(EXTRACTOR_SCHEMA_VERSION >= 32);
    assert_ne!(
        current,
        format!(
            "31:{}:unchanged",
            normalize_path_key(Path::new("/source/lib.rs"))
        )
    );
}
