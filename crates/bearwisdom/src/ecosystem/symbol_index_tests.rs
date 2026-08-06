// =============================================================================
// ecosystem/symbol_index_tests.rs — unit tests for SymbolLocationIndex
// =============================================================================

use super::*;


#[test]
fn insert_and_locate_roundtrip() {
    let mut idx = SymbolLocationIndex::new();
    idx.insert("modernc.org/sqlite", "Open", "/cache/sqlite/sqlite.go");
    let hit = idx.locate("modernc.org/sqlite", "Open");
    assert_eq!(hit, Some(Path::new("/cache/sqlite/sqlite.go")));
}

#[test]
fn miss_returns_none() {
    let idx = SymbolLocationIndex::new();
    assert!(idx.locate("anything", "anything").is_none());
}

#[test]
fn first_writer_wins_on_duplicate() {
    let mut idx = SymbolLocationIndex::new();
    idx.insert("pkg", "Foo", "/a.go");
    idx.insert("pkg", "Foo", "/b.go");
    assert_eq!(idx.locate("pkg", "Foo"), Some(Path::new("/a.go")));
}

#[test]
fn extend_preserves_existing_entries() {
    let mut base = SymbolLocationIndex::new();
    base.insert("pkg", "Foo", "/a.go");
    let mut other = SymbolLocationIndex::new();
    other.insert("pkg", "Foo", "/b.go");
    other.insert("pkg", "Bar", "/c.go");
    base.extend(other);
    assert_eq!(base.locate("pkg", "Foo"), Some(Path::new("/a.go")));
    assert_eq!(base.locate("pkg", "Bar"), Some(Path::new("/c.go")));
    assert_eq!(base.len(), 2);
}

#[test]
fn empty_by_default() {
    let idx = SymbolLocationIndex::new();
    assert!(idx.is_empty());
    assert_eq!(idx.len(), 0);
}

#[test]
fn find_by_name_returns_all_modules_with_match() {
    let mut idx = SymbolLocationIndex::new();
    idx.insert("pkg-a", "Query", "/a/query.rs");
    idx.insert("pkg-b", "Query", "/b/query.rs");
    idx.insert("pkg-a", "Other", "/a/other.rs");

    let mut hits = idx.find_by_name("Query");
    hits.sort_by_key(|(m, _)| *m);
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].0, "pkg-a");
    assert_eq!(hits[0].1, Path::new("/a/query.rs"));
    assert_eq!(hits[1].0, "pkg-b");
    assert_eq!(hits[1].1, Path::new("/b/query.rs"));
}

#[test]
fn find_by_name_empty_when_no_match() {
    let mut idx = SymbolLocationIndex::new();
    idx.insert("pkg", "Foo", "/x.rs");
    assert!(idx.find_by_name("NotThere").is_empty());
}

#[test]
fn extend_accumulates_every_same_key_name_entry() {
    // Two static classes in ONE package offer the same method name. The
    // (module, name) entries map keeps one; the reverse name index must keep
    // BOTH across an extend, or the second class is never locatable.
    let mut child = SymbolLocationIndex::new();
    child.insert("efcore.relational", "HasColumnType", "/dll!!A!!ComplexExtensions");
    child.insert("efcore.relational", "HasColumnType", "/dll!!A!!PropertyExtensions");
    let mut master = SymbolLocationIndex::new();
    master.extend(child);
    let hits = master.find_by_name("HasColumnType");
    assert_eq!(hits.len(), 2, "both offering classes must survive the merge");
}
