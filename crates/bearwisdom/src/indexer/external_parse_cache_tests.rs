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
fn cache_key_is_prefixed_by_the_extractor_digest() {
    let key = cache_key(Path::new("/a/b.php"), "ff");
    assert!(
        key.starts_with(&format!("{EXTRACTOR_SCHEMA_VERSION}:")),
        "the digest must prefix the key so an extractor change flushes every prior entry"
    );
    assert_eq!(EXTRACTOR_SCHEMA_VERSION.len(), 16);
    assert!(EXTRACTOR_SCHEMA_VERSION
        .chars()
        .all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn cache_file_name_is_scoped_to_the_digest() {
    assert_eq!(
        cache_file_name(),
        format!("externals-{EXTRACTOR_SCHEMA_VERSION}.db"),
        "two binaries of different extractor vintages must not share a file"
    );
}

#[test]
fn cache_file_vintage_reads_tagged_tagless_and_companion_names() {
    assert_eq!(cache_file_vintage("externals.db"), Some(""));
    assert_eq!(cache_file_vintage("externals.db-wal"), Some(""));
    assert_eq!(cache_file_vintage("externals-abc.db"), Some("abc"));
    assert_eq!(cache_file_vintage("externals-abc.db-shm"), Some("abc"));
    assert_eq!(cache_file_vintage("index.db"), None);
    assert_eq!(cache_file_vintage("externalsabc.db"), None);
    assert_eq!(cache_file_vintage("externals-abc.txt"), None);
}

#[test]
fn sweep_removes_only_aged_foreign_vintage_files() {
    let dir = tempfile::tempdir().unwrap();
    let now = SystemTime::now();
    let aged = now - STALE_CACHE_AGE - Duration::from_secs(60);
    let write = |name: &str, modified: SystemTime| {
        let path = dir.path().join(name);
        std::fs::write(&path, b"x").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(modified)
            .unwrap();
    };
    write("externals-deadbeefdeadbeef.db", aged);
    write("externals-deadbeefdeadbeef.db-wal", aged);
    write("externals.db", aged);
    write("externals-cafebabecafebabe.db", now);
    write(&cache_file_name(), aged);
    write("index.db", aged);

    sweep_stale_caches(dir.path(), now);

    let exists = |name: &str| dir.path().join(name).exists();
    assert!(
        !exists("externals-deadbeefdeadbeef.db"),
        "an aged foreign vintage is removed"
    );
    assert!(
        !exists("externals-deadbeefdeadbeef.db-wal"),
        "its journal goes with it"
    );
    assert!(
        !exists("externals.db"),
        "the tagless family is a foreign vintage too"
    );
    assert!(
        exists("externals-cafebabecafebabe.db"),
        "a young foreign file may still be in use"
    );
    assert!(
        exists(&cache_file_name()),
        "the current vintage is never swept, whatever its age"
    );
    assert!(
        exists("index.db"),
        "files outside the cache family are untouched"
    );
}
