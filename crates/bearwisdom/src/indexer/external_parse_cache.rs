// =============================================================================
// indexer/external_parse_cache.rs — persistent parse cache for external files
//
// The AssemblyMetadata analog (Roslyn): an external source file is parsed at
// most once per machine. The cache is content-addressed — keyed by
// (extractor_schema_version, absolute_path, sha-256 content_hash) — so a fresh
// reindex, a `--force`, or a different project sharing the same dependency
// skips tree-sitter + the extraction walk (the expensive half) and rebuilds the
// ParsedFile from the stored extraction.
//
// What is cached: only the symbols + refs the lazy-materialization path needs
// (names, kinds, signatures, ref kinds/targets) — NOT the post-extract TypeIds
// (arena-specific, meaningless across runs) nor ref chains/call-args (external
// files are never resolution SOURCES, so their chains are never walked). The
// rebuilt ParsedFile carries empty TypeId/chain fields; the materialized
// symbols use string-based type metadata derived from the cached signatures.
//
// Best-effort: any open/read/write error disables the cache for that op — a
// missing or unwritable cache only costs a re-parse, never correctness.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use once_cell::sync::Lazy;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::types::{
    AliasTarget, EdgeKind, ExtractedRef, ExtractedSymbol, FlowMeta, ParsedFile, SymbolKind,
    Visibility,
};

/// Bumped whenever the cached extraction shape changes. It is part of the key,
/// so a bump makes every prior entry un-matchable (effectively a full flush).
const EXTRACTOR_SCHEMA_VERSION: u32 = 13;

#[cfg(test)]
#[path = "external_parse_cache_tests.rs"]
mod tests;

#[derive(Serialize, Deserialize)]
struct CachedSym {
    name: String,
    qualified_name: String,
    kind: SymbolKind,
    visibility: Option<Visibility>,
    start_line: u32,
    end_line: u32,
    start_col: u32,
    end_col: u32,
    byte_offset: u32,
    signature: Option<String>,
    doc_comment: Option<String>,
    scope_path: Option<String>,
    parent_index: Option<usize>,
}

impl CachedSym {
    fn from_extracted(s: &ExtractedSymbol) -> Self {
        CachedSym {
            name: s.name.clone(),
            qualified_name: s.qualified_name.clone(),
            kind: s.kind,
            visibility: s.visibility,
            start_line: s.start_line,
            end_line: s.end_line,
            start_col: s.start_col,
            end_col: s.end_col,
            byte_offset: s.byte_offset,
            signature: s.signature.clone(),
            doc_comment: s.doc_comment.clone(),
            scope_path: s.scope_path.clone(),
            parent_index: s.parent_index,
        }
    }

    fn into_extracted(self) -> ExtractedSymbol {
        ExtractedSymbol {
            name: self.name,
            qualified_name: self.qualified_name,
            kind: self.kind,
            visibility: self.visibility,
            start_line: self.start_line,
            end_line: self.end_line,
            start_col: self.start_col,
            end_col: self.end_col,
            byte_offset: self.byte_offset,
            signature: self.signature,
            doc_comment: self.doc_comment,
            scope_path: self.scope_path,
            parent_index: self.parent_index,
            // TypeIds are arena-specific and not cached; re-derived from
            // signatures by the materialize path when needed.
            declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct CachedRef {
    source_symbol_index: usize,
    target_name: String,
    kind: EdgeKind,
    line: u32,
    col: u32,
    byte_offset: u32,
    module: Option<String>,
    namespace_segments: Vec<String>,
    is_import_binding: bool,
    is_reexport: bool,
}

impl CachedRef {
    fn from_extracted(r: &ExtractedRef) -> Self {
        CachedRef {
            source_symbol_index: r.source_symbol_index,
            target_name: r.target_name.clone(),
            kind: r.kind,
            line: r.line,
            col: r.col,
            byte_offset: r.byte_offset,
            module: r.module.clone(),
            namespace_segments: r.namespace_segments.clone(),
            is_import_binding: r.is_import_binding,
            is_reexport: r.is_reexport,
        }
    }

    fn into_extracted(self) -> ExtractedRef {
        ExtractedRef {
            source_symbol_index: self.source_symbol_index,
            target_name: self.target_name,
            kind: self.kind,
            line: self.line,
            col: self.col,
            byte_offset: self.byte_offset,
            module: self.module,
            namespace_segments: self.namespace_segments,
            is_import_binding: self.is_import_binding,
            is_reexport: self.is_reexport,
            // Chains / call-args are not cached: external files are never
            // resolution sources, so their refs' chains are never walked.
            chain: None,
            call_args: Vec::new(),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct CachedParse {
    language: String,
    package_id: Option<i64>,
    symbols: Vec<CachedSym>,
    refs: Vec<CachedRef>,
    /// Type-alias targets, qualified by `ts_post_process_external` before the
    /// cache `put`. The chain walker reads these (intersection / mapped /
    /// typeof expansion) to resolve members reached THROUGH an external alias
    /// (`RenderResult`'s `BoundFunctions` branch); dropping them on a cache hit
    /// silently breaks those chains while own-member lookup still works.
    #[serde(default)]
    alias_targets: Vec<(String, AliasTarget)>,
    /// Angular/CSS component selectors `(raw_selector, class_qname)`. Angular
    /// library `.d.ts` carry `ɵɵComponentDeclaration` selectors that back
    /// `selector_qname`; dropping them on a cache hit leaves every `<nb-card>`
    /// template ref unresolved while the class symbol still loads.
    #[serde(default)]
    component_selectors: Vec<(String, String)>,
}

static CACHE: Lazy<Option<Mutex<Connection>>> = Lazy::new(open_cache);

fn open_cache() -> Option<Mutex<Connection>> {
    let dir = std::env::var_os("BEARWISDOM_CACHE_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::cache_dir().map(|d| d.join("bearwisdom")))?;
    std::fs::create_dir_all(&dir).ok()?;
    let conn = Connection::open(dir.join("externals.db")).ok()?;
    // WAL via pragma_update — `execute_batch`/`execute` reject `PRAGMA
    // journal_mode=WAL` because it returns a row. Best-effort: a failed WAL
    // switch just leaves the default rollback journal.
    let _ = conn.pragma_update(None, "journal_mode", "WAL");
    conn.execute(
        "CREATE TABLE IF NOT EXISTS parse_cache (key TEXT PRIMARY KEY, payload TEXT NOT NULL)",
        [],
    )
    .ok()?;
    Some(Mutex::new(conn))
}

fn cache_key(abs_path: &Path, content_hash: &str) -> String {
    format!(
        "{EXTRACTOR_SCHEMA_VERSION}:{}:{content_hash}",
        normalize_path_key(abs_path)
    )
}

/// Canonical, route-independent string for a path used as a cache key. The same
/// logical external file can arrive with different separators (`\` vs `/`) and
/// unfolded `.`/`..` segments depending on which locate / re-export route built
/// the `PathBuf` (`base.join("../dist/./x.d.ts")`). Without normalization the
/// same file keys under several strings, so a warm cache accumulates duplicate
/// rows across runs/binaries and a fresh run materializes a different file set.
/// Fold `.`/`..` and emit `/`-separated segments so one file maps to one key —
/// purely string-based (no filesystem access, works for non-existent paths and
/// resolves no symlinks).
fn normalize_path_key(abs_path: &Path) -> String {
    use std::path::Component;
    let mut prefix = String::new();
    let mut rooted = false;
    let mut segs: Vec<String> = Vec::new();
    for comp in abs_path.components() {
        match comp {
            Component::Prefix(p) => prefix = p.as_os_str().to_string_lossy().replace('\\', "/"),
            Component::RootDir => rooted = true,
            Component::CurDir => {}
            Component::ParentDir => {
                segs.pop();
            }
            Component::Normal(c) => segs.push(c.to_string_lossy().to_string()),
        }
    }
    let body = segs.join("/");
    match (prefix.is_empty(), rooted) {
        (false, true) => format!("{prefix}/{body}"),
        (false, false) => format!("{prefix}{body}"),
        (true, true) => format!("/{body}"),
        (true, false) => body,
    }
}

/// SHA-256 hex of file bytes — the same content hash `parse_file` records.
pub fn content_hash(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// Rebuild a `ParsedFile` from a cache hit, or `None` on miss/error. The
/// `virtual_path` and `size` come from the caller (they are not part of the
/// content-addressed payload). Empty for the fields externals never use.
pub fn get(
    abs_path: &Path,
    content_hash: &str,
    virtual_path: &str,
    size: u64,
) -> Option<ParsedFile> {
    let lock = CACHE.as_ref()?;
    let payload: String = {
        let conn = lock.lock().ok()?;
        conn.query_row(
            "SELECT payload FROM parse_cache WHERE key = ?1",
            [cache_key(abs_path, content_hash)],
            |r| r.get(0),
        )
        .ok()?
    };
    let cp: CachedParse = serde_json::from_str(&payload).ok()?;
    Some(ParsedFile {
        path: virtual_path.to_string(),
        language: cp.language,
        content_hash: content_hash.to_string(),
        size,
        line_count: 0,
        mtime: None,
        package_id: cp.package_id,
        symbols: cp.symbols.into_iter().map(CachedSym::into_extracted).collect(),
        refs: cp.refs.into_iter().map(CachedRef::into_extracted).collect(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: cp.alias_targets,
        component_selectors: cp.component_selectors,
        plugin_flow_emissions: Vec::new(),
    })
}

/// Store a freshly-parsed external file. Best-effort; errors are ignored.
pub fn put(abs_path: &Path, content_hash: &str, pf: &ParsedFile) {
    let Some(lock) = CACHE.as_ref() else { return };
    let Ok(conn) = lock.lock() else { return };
    let cp = CachedParse {
        language: pf.language.clone(),
        package_id: pf.package_id,
        symbols: pf.symbols.iter().map(CachedSym::from_extracted).collect(),
        refs: pf.refs.iter().map(CachedRef::from_extracted).collect(),
        alias_targets: pf.alias_targets.clone(),
        component_selectors: pf.component_selectors.clone(),
    };
    if let Ok(payload) = serde_json::to_string(&cp) {
        let _ = conn.execute(
            "INSERT OR REPLACE INTO parse_cache (key, payload) VALUES (?1, ?2)",
            rusqlite::params![cache_key(abs_path, content_hash), payload],
        );
    }
}
