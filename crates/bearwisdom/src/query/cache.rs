// =============================================================================
// query/cache.rs  —  in-memory LRU cache for common query results
//
// Intended use: check the cache before hitting SQLite for repeated queries
// (IDE hover over the same symbol, MCP tool called with the same args).
//
// Thread-safety: each sub-cache wraps an `LruCache` in a `Mutex`.  This is
// intentionally coarse-grained — these caches are hot paths with tiny critical
// sections (a single hash-map lookup), so per-cache mutexes are cheaper than a
// single RwLock over the whole struct.
//
// Integration: wired into `DbPool` as an optional `Arc<QueryCache>` field so
// it is shared across all pool connections.  Query functions are not yet
// modified to use it; that is incremental adoption work.  Use
// `DbPool::cache()` to get the shared instance.
// =============================================================================

use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::Mutex;

// ---------------------------------------------------------------------------
// QueryCache
// ---------------------------------------------------------------------------

/// Thread-safe, per-kind LRU cache for serialised query results.
///
/// Each sub-cache stores JSON strings so the cache layer is decoupled from
/// the concrete result types.  Callers serialise before storing and
/// deserialise after retrieval; this keeps the cache generic and avoids
/// requiring result types to implement `Clone`.
pub struct QueryCache {
    /// `symbol_info` cache keyed by qualified name → serialised JSON result.
    symbol_info: Mutex<LruCache<String, String>>,
    /// `references` cache keyed by target name → serialised JSON result.
    references: Mutex<LruCache<String, String>>,
    /// `search` cache keyed by raw query string → serialised JSON result.
    search: Mutex<LruCache<String, String>>,
    /// `architecture` cache — single entry (keyed by "default").
    architecture: Mutex<LruCache<String, String>>,
}

impl QueryCache {
    /// Create a new cache with the given per-kind capacity.
    ///
    /// `capacity` must be ≥ 1; values below that are silently clamped to 1.
    pub fn new(capacity: usize) -> Self {
        let cap = NonZeroUsize::new(capacity)
            .unwrap_or_else(|| NonZeroUsize::new(1).expect("1 is non-zero"));
        Self {
            symbol_info: Mutex::new(LruCache::new(cap)),
            references: Mutex::new(LruCache::new(cap)),
            search: Mutex::new(LruCache::new(cap)),
            architecture: Mutex::new(LruCache::new(NonZeroUsize::new(1).unwrap())),
        }
    }

    // ── symbol_info ──────────────────────────────────────────────────────────

    /// Look up a cached `symbol_info` result by qualified name.
    ///
    /// Returns `None` if the entry is absent or the mutex is poisoned.
    pub fn get_symbol_info(&self, key: &str) -> Option<String> {
        self.symbol_info.lock().ok()?.get(key).cloned()
    }

    /// Store a `symbol_info` result.  Silently drops the value if the mutex
    /// is poisoned.
    pub fn put_symbol_info(&self, key: String, value: String) {
        if let Ok(mut cache) = self.symbol_info.lock() {
            cache.put(key, value);
        }
    }

    // ── references ───────────────────────────────────────────────────────────

    /// Look up a cached `references` result by target name.
    pub fn get_references(&self, target_name: &str) -> Option<String> {
        self.references.lock().ok()?.get(target_name).cloned()
    }

    /// Store a `references` result keyed by target name.
    pub fn put_references(&self, target_name: String, value: String) {
        if let Ok(mut cache) = self.references.lock() {
            cache.put(target_name, value);
        }
    }

    // ── architecture ────────────────────────────────────────────────────────

    /// Look up the cached architecture overview.
    pub fn get_architecture(&self) -> Option<String> {
        self.architecture.lock().ok()?.get("default").cloned()
    }

    /// Store the architecture overview result.
    pub fn put_architecture(&self, value: String) {
        if let Ok(mut cache) = self.architecture.lock() {
            cache.put("default".to_string(), value);
        }
    }

    // ── search ───────────────────────────────────────────────────────────────

    /// Look up a cached `search` result by raw query string.
    pub fn get_search(&self, query: &str) -> Option<String> {
        self.search.lock().ok()?.get(query).cloned()
    }

    /// Store a `search` result keyed by raw query string.
    pub fn put_search(&self, query: String, value: String) {
        if let Ok(mut cache) = self.search.lock() {
            cache.put(query, value);
        }
    }

    // ── invalidation ─────────────────────────────────────────────────────────

    /// Invalidate all sub-caches.  Call after a full reindex.
    pub fn invalidate_all(&self) {
        if let Ok(mut c) = self.symbol_info.lock() {
            c.clear();
        }
        if let Ok(mut c) = self.references.lock() {
            c.clear();
        }
        if let Ok(mut c) = self.search.lock() {
            c.clear();
        }
        if let Ok(mut c) = self.architecture.lock() {
            c.clear();
        }
    }

    /// Invalidate caches after a set of files changed.
    ///
    /// Fine-grained per-file invalidation is future work; for now this
    /// delegates to `invalidate_all` because any symbol in any cached result
    /// may belong to one of the changed files.
    pub fn invalidate_files(&self, _paths: &[String]) {
        self.invalidate_all();
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_symbol_info_roundtrip() {
        let cache = QueryCache::new(10);
        assert!(cache.get_symbol_info("Foo.Bar").is_none());

        cache.put_symbol_info("Foo.Bar".to_string(), r#"{"name":"Bar"}"#.to_string());
        assert_eq!(
            cache.get_symbol_info("Foo.Bar"),
            Some(r#"{"name":"Bar"}"#.to_string())
        );
    }

    #[test]
    fn test_references_roundtrip() {
        let cache = QueryCache::new(10);
        assert!(cache.get_references("Foo.Bar").is_none());

        cache.put_references("Foo.Bar".to_string(), r#"[{"file":"a.cs"}]"#.to_string());
        assert_eq!(
            cache.get_references("Foo.Bar"),
            Some(r#"[{"file":"a.cs"}]"#.to_string())
        );
    }

    #[test]
    fn test_search_roundtrip() {
        let cache = QueryCache::new(10);
        assert!(cache.get_search("FooController").is_none());

        cache.put_search(
            "FooController".to_string(),
            r#"[{"name":"FooController"}]"#.to_string(),
        );
        assert_eq!(
            cache.get_search("FooController"),
            Some(r#"[{"name":"FooController"}]"#.to_string())
        );
    }

    #[test]
    fn test_invalidate_all_clears_all_caches() {
        let cache = QueryCache::new(10);
        cache.put_symbol_info("A.B".to_string(), "{}".to_string());
        cache.put_references("X.Y".to_string(), "[]".to_string());
        cache.put_search("q".to_string(), "[]".to_string());

        cache.invalidate_all();

        assert!(cache.get_symbol_info("A.B").is_none());
        assert!(cache.get_references("X.Y").is_none());
        assert!(cache.get_search("q").is_none());
    }

    #[test]
    fn test_lru_eviction() {
        // capacity = 2: third insertion must evict the LRU entry.
        let cache = QueryCache::new(2);
        cache.put_symbol_info("A".to_string(), "a".to_string());
        cache.put_symbol_info("B".to_string(), "b".to_string());
        // "A" is now the LRU entry — accessing "B" promotes it.
        let _ = cache.get_symbol_info("B");
        // Insert "C"; LRU ("A") should be evicted.
        cache.put_symbol_info("C".to_string(), "c".to_string());

        assert!(cache.get_symbol_info("A").is_none(), "A should have been evicted");
        assert!(cache.get_symbol_info("B").is_some());
        assert!(cache.get_symbol_info("C").is_some());
    }

    #[test]
    fn test_invalidate_files_delegates_to_invalidate_all() {
        let cache = QueryCache::new(10);
        cache.put_symbol_info("A.B".to_string(), "{}".to_string());

        cache.invalidate_files(&["src/foo.cs".to_string()]);

        assert!(cache.get_symbol_info("A.B").is_none());
    }

    #[test]
    fn test_capacity_one() {
        // Edge case: capacity=0 is clamped to 1.
        let cache = QueryCache::new(0);
        cache.put_symbol_info("X".to_string(), "x".to_string());
        assert!(cache.get_symbol_info("X").is_some());
    }
}
