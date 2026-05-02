// =============================================================================
// services.rs — per-project IndexService cache for the MCP server
//
// Lets a single MCP instance serve multiple projects. The first tool call for
// a project lazily opens an `IndexService` (which spawns the file watcher and
// owns the connection pool); subsequent calls reuse it. Eviction is LRU-bounded
// so the watcher count stays predictable across long sessions.
//
// Failure modes:
//   * `PROJECT_NOT_FOUND` — the path doesn't exist or isn't a directory.
//     Returned as a structured MCP error so callers can recover by passing a
//     correct path. Auto-creating an index dir for a typo would be silent and
//     awful; we refuse instead.
// =============================================================================

use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use lru::LruCache;
use tracing::{debug, info, warn};

use bearwisdom::{IndexService, IndexServiceOptions};

/// Thread-safe cache of `IndexService` instances keyed by canonicalised
/// project root. Lookups bump the LRU order; evicted services drop their
/// watcher and pool when the cache releases its `Arc`.
pub struct ServiceCache {
    inner: Mutex<LruCache<PathBuf, Arc<IndexService>>>,
    options: IndexServiceOptions,
}

impl ServiceCache {
    /// Build a new cache with the given capacity and shared service options.
    pub fn new(capacity: usize, options: IndexServiceOptions) -> Self {
        let cap = NonZeroUsize::new(capacity.max(1))
            .expect("capacity is clamped to >= 1 above");
        Self {
            inner: Mutex::new(LruCache::new(cap)),
            options,
        }
    }

    /// Insert a pre-built service under its project root. Used at startup to
    /// seed the default project so the watcher is up before any tool call.
    /// The key is canonicalised to match the lookup path in `get_or_open`.
    pub fn insert(&self, project: PathBuf, service: Arc<IndexService>) {
        let canonical = project.canonicalize().unwrap_or(project);
        let mut guard = self.inner.lock().expect("ServiceCache mutex poisoned");
        if let Some(evicted_key) = guard.push(canonical, service).map(|(k, _)| k) {
            info!(
                "ServiceCache: evicted {} on insert (cap reached)",
                evicted_key.display()
            );
        }
    }

    /// Return the service for `project`, opening it lazily on first touch.
    ///
    /// Errors as a `(code, message)` pair the caller can wrap in the MCP
    /// error-response shape:
    ///   * `PROJECT_NOT_FOUND` if `project` is missing or not a directory.
    ///   * `INTERNAL_ERROR` if `IndexService::open` fails (DB / watcher).
    pub fn get_or_open(&self, project: &Path) -> Result<Arc<IndexService>, (String, String)> {
        let canonical = project.canonicalize().unwrap_or_else(|_| project.to_path_buf());

        if let Some(svc) = self.peek(&canonical) {
            return Ok(svc);
        }

        if !canonical.is_dir() {
            return Err((
                "PROJECT_NOT_FOUND".to_string(),
                format!(
                    "project path does not exist or is not a directory: {}",
                    canonical.display()
                ),
            ));
        }

        let db_path = bearwisdom::resolve_db_path(&canonical)
            .map_err(|e| ("INTERNAL_ERROR".to_string(), format!("resolve db path: {e}")))?;
        let svc = IndexService::open(&db_path, &canonical, self.options.clone())
            .map_err(|e| ("INTERNAL_ERROR".to_string(), format!("open IndexService: {e:#}")))?;
        let arc = Arc::new(svc);

        let mut guard = self.inner.lock().expect("ServiceCache mutex poisoned");
        // Lost-update guard — another caller may have raced to open the same
        // project between our peek and insert. If so, drop our copy and use
        // theirs. This is rare (single-threaded MCP transport in practice)
        // but cheap to handle correctly.
        if let Some(existing) = guard.get(&canonical) {
            debug!(
                "ServiceCache: race on {} — keeping existing service",
                canonical.display()
            );
            return Ok(existing.clone());
        }
        if let Some(evicted_key) = guard.push(canonical.clone(), arc.clone()).map(|(k, _)| k) {
            info!(
                "ServiceCache: evicted {} on lazy-open (cap reached)",
                evicted_key.display()
            );
        }
        info!("ServiceCache: opened {}", canonical.display());
        Ok(arc)
    }

    /// Lookup helper that bumps LRU order. Public so the seed-then-resolve
    /// path can avoid double-locking.
    fn peek(&self, project: &Path) -> Option<Arc<IndexService>> {
        let mut guard = self.inner.lock().expect("ServiceCache mutex poisoned");
        guard.get(project).cloned()
    }

    /// Number of cached services. Test/diagnostic helper.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.inner.lock().map(|g| g.len()).unwrap_or(0)
    }
}

impl Drop for ServiceCache {
    fn drop(&mut self) {
        // Best-effort log so a session shutdown shows watcher cleanup.
        if let Ok(g) = self.inner.lock() {
            if g.len() > 0 {
                warn!("ServiceCache: dropping {} cached services", g.len());
            }
        }
    }
}
