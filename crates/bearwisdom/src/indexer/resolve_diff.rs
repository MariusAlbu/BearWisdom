// =============================================================================
// indexer/resolve_diff — the parity oracle
//
// Indexes a project twice into separate in-memory databases — once through the
// legacy engine, once through the new engine — and diffs their resolved edge
// sets. The parse, symbol extraction, and symbol writes are identical across
// both runs (see `full_index_inner`); only the per-ref bind decision differs.
// So the edge-set difference is exactly the resolution gap between the engines:
//
//   regressions = edges the legacy engine bound that the new engine did not
//   gains       = edges the new engine bound that the legacy engine did not
//
// `regressions` is the parity worklist: each row names a (source, target, kind)
// relationship the new engine must learn to bind before legacy can be deleted.
//
// Edges are keyed by qualified name, not symbol id: the two runs assign ids
// independently, but the symbol qnames are identical, so a qname-keyed edge is
// directly comparable across runs.
// =============================================================================

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};

use crate::db::Database;
use crate::indexer::full::{full_index, full_index_engine};

/// A resolved edge identified by its endpoints' qualified names and kind —
/// comparable across two independent index runs whose symbol ids differ.
#[derive(Hash, PartialEq, Eq, Clone, Debug)]
pub struct EdgeKey {
    pub source: String,
    pub target: String,
    pub kind: String,
}

/// The result of diffing the two engines' resolved-edge sets for one project.
#[derive(Debug)]
pub struct ResolveDiff {
    /// Distinct (source, target, kind) edges the legacy engine resolved.
    pub legacy_edges: usize,
    /// Distinct (source, target, kind) edges the new engine resolved.
    pub engine_edges: usize,
    /// Edges legacy bound that the new engine did not — the parity worklist.
    pub regressions: Vec<EdgeKey>,
    /// Edges the new engine bound that legacy did not.
    pub gains: Vec<EdgeKey>,
}

impl ResolveDiff {
    /// `true` when the new engine resolves every edge the legacy engine did.
    pub fn at_parity(&self) -> bool {
        self.regressions.is_empty()
    }
}

/// Index `project_root` with both engines and diff their resolved edges.
pub fn resolve_diff(project_root: &Path) -> Result<ResolveDiff> {
    let legacy = index_edges(project_root, false).context("legacy index")?;
    let engine = index_edges(project_root, true).context("engine index")?;
    let (regressions, gains) = diff_sets(&legacy, &engine);
    Ok(ResolveDiff {
        legacy_edges: legacy.len(),
        engine_edges: engine.len(),
        regressions,
        gains,
    })
}

/// `(legacy − engine, engine − legacy)`, each sorted for stable output.
fn diff_sets(
    legacy: &HashSet<EdgeKey>,
    engine: &HashSet<EdgeKey>,
) -> (Vec<EdgeKey>, Vec<EdgeKey>) {
    let mut regressions: Vec<EdgeKey> = legacy.difference(engine).cloned().collect();
    let mut gains: Vec<EdgeKey> = engine.difference(legacy).cloned().collect();
    regressions.sort_unstable_by(|a, b| edge_order(a).cmp(&edge_order(b)));
    gains.sort_unstable_by(|a, b| edge_order(a).cmp(&edge_order(b)));
    (regressions, gains)
}

fn edge_order(e: &EdgeKey) -> (&str, &str, &str) {
    (&e.source, &e.target, &e.kind)
}

/// Full-index `project_root` into a fresh in-memory database via the selected
/// engine and return its resolved edges keyed by qname.
fn index_edges(project_root: &Path, use_engine: bool) -> Result<HashSet<EdgeKey>> {
    let mut db = Database::open_in_memory().context("open in-memory db")?;
    if use_engine {
        full_index_engine(&mut db, project_root, None, None, None)?;
    } else {
        full_index(&mut db, project_root, None, None, None)?;
    }
    read_internal_edges(&db)
}

/// Read the resolved edges whose source is an internal (project) symbol, keyed
/// by `(source_qname, target_qname, kind)`. The resolve loop only writes
/// internal-source edges, so the `origin` filter is a guard, not a narrowing.
fn read_internal_edges(db: &Database) -> Result<HashSet<EdgeKey>> {
    let conn = db.conn();
    let mut stmt = conn.prepare(
        "SELECT s.qualified_name, t.qualified_name, e.kind
           FROM edges e
           JOIN symbols s ON e.source_id = s.id
           JOIN symbols t ON e.target_id = t.id
          WHERE s.origin = 'internal'",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(EdgeKey {
            source: r.get(0)?,
            target: r.get(1)?,
            kind: r.get(2)?,
        })
    })?;
    let mut set = HashSet::new();
    for row in rows {
        set.insert(row.context("read edge row")?);
    }
    Ok(set)
}

#[cfg(test)]
#[path = "resolve_diff_tests.rs"]
mod tests;
