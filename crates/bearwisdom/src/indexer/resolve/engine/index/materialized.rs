// =============================================================================
// indexer/resolve/engine/index/materialized.rs — lazily-materialized external store
//
// The append-only store that backs lazy external materialization. When a
// lookup misses the eager-internal maps, the external location index points at
// a file; that file is parsed once and its symbols are interned HERE, under a
// shared `&self` borrow across rayon workers. The store hands back `&SymbolInfo`
// references that stay valid forever after the push — that is what makes a
// materialized symbol reusable by any later reference, in any file, on any
// worker, without re-deriving it.
//
// Why these types:
//   * `boxcar::Vec` — lock-free append under `&self` with STABLE element
//     addresses. A reference handed to one worker stays valid while another
//     worker pushes. This is the load-bearing property; a plain `Vec` would
//     reallocate and invalidate outstanding references.
//   * `DashMap` — concurrent name/qname/parent indices, sharded so workers
//     interning different files don't contend on one lock.
//   * `OnceCell` per file — one worker parses each external file; concurrent
//     hitters block briefly then reuse, so a file is never parsed twice.
//
// Append-only is the safety invariant: nothing is ever mutated or removed, so
// no reader's borrow is invalidated and no worker shadows an eager-internal
// symbol (the lookup checks the eager store first).
// =============================================================================

use std::path::PathBuf;
use std::sync::Arc;

use dashmap::DashMap;
use once_cell::sync::OnceCell;

use crate::indexer::resolve::engine::SymbolInfo;

/// A stable handle into one of the two symbol stores. `Internal` indexes the
/// eager, contiguous build-time store; `Materialized` indexes the append-only
/// `boxcar` store. Both indices are stable for the lifetime of the index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolRef {
    Internal(u32),
    Materialized(u32),
}

/// Append-only store of materialized external symbols plus the concurrent
/// indices that answer lookups against them. Shared by `&self` across the
/// resolve pass's rayon workers.
#[derive(Default)]
pub struct MaterializedStore {
    /// The symbols themselves. `boxcar::Vec` gives stable `&SymbolInfo` after
    /// a `&self` push, so an index into it is a permanent handle.
    store: boxcar::Vec<SymbolInfo>,
    /// Simple name → store indices.
    by_name: DashMap<Box<str>, Vec<u32>>,
    /// Qualified name → store index. First writer wins (mirrors the eager
    /// `by_qname` first-wins semantics).
    by_qname: DashMap<Box<str>, u32>,
    /// Parent qualified name → member store indices.
    by_parent: DashMap<Box<str>, Vec<u32>>,
    /// Per-file single-materialization guard. `get_or_init` ensures one worker
    /// parses + interns each external file; others reuse.
    file_guards: DashMap<PathBuf, Arc<OnceCell<()>>>,
}

impl MaterializedStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether anything has been materialized yet. The common internal-only
    /// project keeps this `true` for the whole pass, so lookups skip the
    /// cross-store merge entirely.
    pub fn is_empty(&self) -> bool {
        self.store.count() == 0
    }

    /// Number of materialized symbols — diagnostic only.
    pub fn len(&self) -> usize {
        self.store.count()
    }

    /// Intern one symbol, registering it under its simple name, qualified
    /// name, and parent qname. Returns the stable store index. `&self` push:
    /// callable from inside the rayon resolve pass.
    pub fn intern(&self, sym: SymbolInfo) -> u32 {
        let name: Box<str> = sym.name.as_str().into();
        let qname: Box<str> = sym.qualified_name.as_str().into();
        let parent: Box<str> = parent_qname(&sym.qualified_name).into();
        let idx = self.store.push(sym) as u32;
        self.by_name.entry(name).or_default().push(idx);
        self.by_qname.entry(qname).or_insert(idx);
        self.by_parent.entry(parent).or_default().push(idx);
        idx
    }

    /// Borrow a materialized symbol by its stable index.
    pub fn get(&self, idx: u32) -> Option<&SymbolInfo> {
        self.store.get(idx as usize)
    }

    /// All materialized symbols with the given simple name. Empty in the
    /// common internal-only case.
    pub fn by_name(&self, name: &str) -> Vec<&SymbolInfo> {
        self.collect(self.by_name.get(name).map(|r| r.clone()))
    }

    /// The materialized symbol with this exact qualified name, if any.
    pub fn by_qualified_name(&self, qname: &str) -> Option<&SymbolInfo> {
        let idx = *self.by_qname.get(qname)?;
        self.store.get(idx as usize)
    }

    /// Direct members of a materialized parent type.
    pub fn members_of(&self, parent_qname: &str) -> Vec<&SymbolInfo> {
        self.collect(self.by_parent.get(parent_qname).map(|r| r.clone()))
    }

    /// Resolve a copied index list against the stable store. The DashMap guard
    /// is dropped (via the `Option<Vec<u32>>` copy) before indexing the store,
    /// so a reader never holds a shard lock while dereferencing.
    fn collect(&self, idxs: Option<Vec<u32>>) -> Vec<&SymbolInfo> {
        match idxs {
            Some(idxs) => idxs
                .into_iter()
                .filter_map(|i| self.store.get(i as usize))
                .collect(),
            None => Vec::new(),
        }
    }

    /// The single-materialization guard for `file`. One worker runs the init
    /// closure; concurrent callers for the same file block until it completes,
    /// then observe the cached unit. Caller threads the actual parse/intern
    /// through the closure.
    pub fn file_guard(&self, file: &std::path::Path) -> Arc<OnceCell<()>> {
        self.file_guards
            .entry(file.to_path_buf())
            .or_insert_with(|| Arc::new(OnceCell::new()))
            .clone()
    }
}

/// The parent qualified name of `qname` — everything up to the last `.`
/// separator, or the empty string for a top-level name. Mirrors the eager
/// `members_by_parent` keying so a materialized member lands under the same
/// parent key a lookup probes.
fn parent_qname(qname: &str) -> &str {
    match qname.rfind('.') {
        Some(dot) => &qname[..dot],
        None => "",
    }
}

#[cfg(test)]
#[path = "materialized_tests.rs"]
mod tests;
