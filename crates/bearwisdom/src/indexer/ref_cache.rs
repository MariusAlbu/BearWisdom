// =============================================================================
// indexer/ref_cache.rs  —  per-file symbol/ref cache for incremental reindex
//
// After a full index the parsed symbols and refs for every file are stored
// here (keyed by file path + content hash).  On incremental reindex, the
// blast-radius pass currently re-parses dependent files from disk to obtain
// their refs.  With this cache, unchanged dependent files can skip the
// tree-sitter parse and use the pre-cached data instead.
//
// Memory trade-off: caching all symbols + refs for a large project is
// non-trivial.  `ExtractedSymbol` and `ExtractedRef` do not hold file
// content, so the footprint is proportional to the number of symbols/refs
// rather than file size.  For a 1000-file C# project (~30k symbols, ~100k
// refs) this is roughly 50–150 MB depending on string lengths — acceptable
// for a long-running daemon process (MCP server, file watcher).  The cache
// is disabled by default (`Database::ref_cache` is `None`) and must be
// explicitly opt-in by callers that need the fast incremental path.
//
// Integration points:
//   • `full_index` calls `RefCache::store_all` after parsing.
//   • `reindex_files` (blast-radius pass) calls `RefCache::get` to skip
//     re-parsing unchanged dependent files.
// =============================================================================

use crate::types::{ExtractedRef, ExtractedSymbol, ParsedFile};
use rustc_hash::FxHashMap;

// ---------------------------------------------------------------------------
// CachedFile — per-file snapshot
// ---------------------------------------------------------------------------

struct CachedFile {
    /// SHA-256 hex digest of the file content at the time of caching.
    hash: String,
    /// Symbols extracted from this file.
    symbols: Vec<ExtractedSymbol>,
    /// Refs (unresolved references) extracted from this file.
    refs: Vec<ExtractedRef>,
}

// ---------------------------------------------------------------------------
// RefCache
// ---------------------------------------------------------------------------

/// In-memory cache of parsed symbol and ref data, keyed by file path.
///
/// Call [`store_all`](RefCache::store_all) after a full index to populate the
/// cache.  Then call [`get`](RefCache::get) during incremental reindex to
/// retrieve unchanged files without re-parsing.
#[derive(Default)]
pub struct RefCache {
    /// `relative_file_path` → cached parse output.
    cache: FxHashMap<String, CachedFile>,
}

impl RefCache {
    /// Create an empty cache.
    pub fn new() -> Self {
        Self {
            cache: FxHashMap::default(),
        }
    }

    // ── Single-file operations ────────────────────────────────────────────────

    /// Store the parsed symbols and refs for `path` with content hash `hash`.
    ///
    /// If an entry already exists for `path` it is replaced unconditionally.
    pub fn store(&mut self, path: &str, hash: &str, file: &ParsedFile) {
        self.cache.insert(
            path.to_string(),
            CachedFile {
                hash: hash.to_string(),
                symbols: file.symbols.clone(),
                refs: file.refs.clone(),
            },
        );
    }

    /// Retrieve cached symbols and refs for `path` if `current_hash` matches
    /// the hash stored at index time (i.e. the file has not changed on disk).
    ///
    /// Returns `None` when:
    /// - the path has never been cached, or
    /// - the hash differs (file was modified since the last full index).
    pub fn get<'a>(
        &'a self,
        path: &str,
        current_hash: &str,
    ) -> Option<(&'a [ExtractedSymbol], &'a [ExtractedRef])> {
        let cached = self.cache.get(path)?;
        if cached.hash == current_hash {
            Some((&cached.symbols, &cached.refs))
        } else {
            None
        }
    }

    /// Remove cached data for `path`.  Call when a file is modified or
    /// deleted so stale data is not returned on the next incremental pass.
    pub fn invalidate(&mut self, path: &str) {
        self.cache.remove(path);
    }

    // ── Bulk operations ───────────────────────────────────────────────────────

    /// Store all parsed files from a full index in one call.
    ///
    /// Each `ParsedFile` supplies its own `path` and `content_hash`; no extra
    /// arguments required.
    pub fn store_all(&mut self, parsed: &[ParsedFile]) {
        for pf in parsed {
            self.store(&pf.path, &pf.content_hash, pf);
        }
    }

    /// Number of entries currently in the cache.
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    /// Returns `true` if the cache contains no entries.
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{EdgeKind, ParsedFile, SymbolKind};

    fn make_parsed_file(path: &str, hash: &str, symbol_name: &str) -> ParsedFile {
        ParsedFile {
            path: path.to_string(),
            language: "csharp".to_string(),
            content_hash: hash.to_string(),
            size: 100,
            line_count: 10,
            mtime: None,
            package_id: None,
            symbols: vec![crate::types::ExtractedSymbol {
                name: symbol_name.to_string(),
                qualified_name: format!("Root.{symbol_name}"),
                kind: SymbolKind::Class,
                visibility: None,
                start_line: 0,
                end_line: 5,
                start_col: 0,
                end_col: 1,
                signature: None,
                doc_comment: None,
                scope_path: None,
                parent_index: None,
            }],
            refs: vec![crate::types::ExtractedRef {
                source_symbol_index: 0,
                target_name: "OtherClass".to_string(),
                kind: EdgeKind::TypeRef,
                line: 2,
                module: None,
                chain: None,
                byte_offset: 0,
                            namespace_segments: Vec::new(),
                            call_args: Vec::new(),
}],
            routes: vec![],
            db_sets: vec![],
            symbol_origin_languages: vec![],
            ref_origin_languages: vec![],
            symbol_from_snippet: vec![],
            content: None,
            has_errors: false,
            flow: crate::types::FlowMeta::default(),
            connection_points: Vec::new(),
            demand_contributions: Vec::new(),
            alias_targets: Vec::new(),
            component_selectors: Vec::new(),
        }
    }

    #[test]
    fn test_store_and_get_matching_hash() {
        let mut cache = RefCache::new();
        let pf = make_parsed_file("src/Foo.cs", "abc123", "Foo");
        cache.store("src/Foo.cs", "abc123", &pf);

        let result = cache.get("src/Foo.cs", "abc123");
        assert!(result.is_some());
        let (syms, refs) = result.unwrap();
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "Foo");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].target_name, "OtherClass");
    }

    #[test]
    fn test_get_returns_none_for_hash_mismatch() {
        let mut cache = RefCache::new();
        let pf = make_parsed_file("src/Foo.cs", "abc123", "Foo");
        cache.store("src/Foo.cs", "abc123", &pf);

        // Different hash — file changed on disk.
        assert!(cache.get("src/Foo.cs", "deadbeef").is_none());
    }

    #[test]
    fn test_get_returns_none_for_unknown_path() {
        let cache = RefCache::new();
        assert!(cache.get("src/Unknown.cs", "abc123").is_none());
    }

    #[test]
    fn test_invalidate_removes_entry() {
        let mut cache = RefCache::new();
        let pf = make_parsed_file("src/Bar.cs", "hash1", "Bar");
        cache.store("src/Bar.cs", "hash1", &pf);
        assert!(cache.get("src/Bar.cs", "hash1").is_some());

        cache.invalidate("src/Bar.cs");
        assert!(cache.get("src/Bar.cs", "hash1").is_none());
    }

    #[test]
    fn test_store_all_populates_from_vec() {
        let mut cache = RefCache::new();
        let files = vec![
            make_parsed_file("src/A.cs", "h1", "A"),
            make_parsed_file("src/B.cs", "h2", "B"),
        ];

        cache.store_all(&files);

        assert_eq!(cache.len(), 2);
        assert!(cache.get("src/A.cs", "h1").is_some());
        assert!(cache.get("src/B.cs", "h2").is_some());
    }

    #[test]
    fn test_store_overwrites_existing_entry() {
        let mut cache = RefCache::new();
        let old = make_parsed_file("src/C.cs", "old_hash", "OldName");
        cache.store("src/C.cs", "old_hash", &old);

        let new = make_parsed_file("src/C.cs", "new_hash", "NewName");
        cache.store("src/C.cs", "new_hash", &new);

        // Old hash no longer valid.
        assert!(cache.get("src/C.cs", "old_hash").is_none());
        // New hash returns the updated entry.
        let (syms, _) = cache.get("src/C.cs", "new_hash").unwrap();
        assert_eq!(syms[0].name, "NewName");
    }

    #[test]
    fn test_len_and_is_empty() {
        let mut cache = RefCache::new();
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);

        let pf = make_parsed_file("src/D.cs", "h", "D");
        cache.store("src/D.cs", "h", &pf);
        assert!(!cache.is_empty());
        assert_eq!(cache.len(), 1);
    }
}
