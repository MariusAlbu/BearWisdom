// =============================================================================
// indexer/resolve/synthesize_dispatch.rs — Dispatch-candidate synthesis (L2)
//
// Closes the biggest static/runtime gap in the reachability model: virtual
// dispatch. A method on a trait / interface / abstract class with at least
// one same-named override in a subclass needs an edge from the parent
// method to each override — otherwise the reachability BFS reaches the
// trait method (from real callers) but never the impl, and every impl
// looks dead even when the framework dispatches to it at runtime.
//
// v1 algorithm: broad fan-out. For every (parent type T, child type S)
// inheritance pair (transitively via `Inherits`/`Implements` edges), find
// methods on T whose name also exists on S and emit one
// `dispatch_candidate` edge `T.method → S.method`. Confidence 0.6 — above
// the reachability threshold (0.5) so the BFS traverses these, but below
// real resolver-emitted edges (≥ 0.8) so the trust signal differentiates.
//
// Refinements deferred to later phases:
//   • Type narrowing (TS-flow): when the call site narrows the receiver
//     to a concrete subtype, emit only that subtype's dispatch_candidate
//     instead of the whole subclass fan-out.
//   • Arity / signature matching: today same-name is enough; later we
//     can require parameter-count parity to prune overload collisions.
// =============================================================================

use crate::db::Database;
use anyhow::{Context, Result};

/// Confidence stamped on synthesized dispatch edges. Above the BFS
/// threshold (0.5) so reachability traverses them, below ground-truth
/// resolver edges (≥ 0.8) so query callers can tell them apart.
pub const DISPATCH_CANDIDATE_CONFIDENCE: f64 = 0.6;

/// Sentinel `source_line` value for synthesized dispatch edges. Real
/// edges record the line where the call site appears; synthesized
/// dispatch has no call site, so we stamp 0 so the
/// `UNIQUE(source_id, target_id, kind, source_line)` constraint
/// catches duplicates across re-runs.
const SYNTH_SOURCE_LINE: i64 = 0;

/// Strategy tag for the `edges.strategy` column. Lets diagnostics queries
/// answer "where did this edge come from?" without rerunning resolution.
const SYNTH_STRATEGY: &str = "synth_dispatch";

/// Build dispatch_candidate edges from the current inheritance graph.
/// Idempotent — clears prior synthesized rows before re-emitting.
/// Returns the count of edges inserted (after `INSERT OR IGNORE` dedupe).
///
/// Run in `finalize_resolution` BEFORE `materialize_reachability` so the
/// BFS sees the synthesized edges in the same pass.
pub fn synthesize_dispatch_edges(db: &Database) -> Result<usize> {
    let conn = db.conn();

    let tx = conn
        .unchecked_transaction()
        .context("synth_dispatch: begin transaction")?;

    // Clear stale rows from prior runs. The UNIQUE constraint on
    // (source_id, target_id, kind, source_line) means a stale row would
    // be quietly retained by `INSERT OR IGNORE` even when the underlying
    // type relationship has been deleted.
    tx.execute("DELETE FROM edges WHERE kind = 'dispatch_candidate'", [])
        .context("synth_dispatch: clear stale dispatch edges")?;

    // Single SQL pass: transitive inheritance closure × same-named method
    // pairs. SQLite's recursive CTE uses UNION (not UNION ALL) to
    // deduplicate (child, ancestor) pairs and terminate on diamond
    // hierarchies without an explicit visited-set in Rust.
    //
    // parent_t.origin is intentionally NOT filtered to 'internal':
    // calls into external trait methods (e.g. an internal class extending
    // an external library's base class) still need to dispatch to the
    // internal override. child_t.origin = 'internal' keeps the inserted
    // rows scoped to project-owned impl symbols.
    let inserted = tx
        .execute(
            "INSERT OR IGNORE INTO edges \
                (source_id, target_id, kind, source_line, confidence, strategy) \
             WITH RECURSIVE ancestors(child_id, ancestor_id) AS ( \
                 SELECT source_id AS child_id, target_id AS ancestor_id \
                 FROM edges \
                 WHERE kind IN ('inherits', 'implements') \
                 UNION \
                 SELECT a.child_id, e.target_id \
                 FROM ancestors a \
                 JOIN edges e ON e.source_id = a.ancestor_id \
                 WHERE e.kind IN ('inherits', 'implements') \
             ) \
             SELECT DISTINCT \
                 trait_m.id, \
                 impl_m.id, \
                 'dispatch_candidate', \
                 ?1, \
                 ?2, \
                 ?3 \
             FROM ancestors anc \
             JOIN symbols parent_t ON parent_t.id = anc.ancestor_id \
             JOIN symbols child_t  ON child_t.id  = anc.child_id \
             JOIN symbols trait_m ON trait_m.scope_path = parent_t.qualified_name \
                                  AND trait_m.kind IN ('method', 'function') \
             JOIN symbols impl_m  ON impl_m.scope_path  = child_t.qualified_name \
                                  AND impl_m.kind IN ('method', 'function') \
                                  AND impl_m.name = trait_m.name \
             WHERE parent_t.kind IN ('trait', 'interface', 'class', \
                                     'abstract_class', 'protocol') \
               AND child_t.origin = 'internal' \
               AND trait_m.id != impl_m.id",
            rusqlite::params![
                SYNTH_SOURCE_LINE,
                DISPATCH_CANDIDATE_CONFIDENCE,
                SYNTH_STRATEGY,
            ],
        )
        .context("synth_dispatch: insert dispatch_candidate edges")?;

    tx.commit().context("synth_dispatch: commit")?;
    Ok(inserted)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "synthesize_dispatch_tests.rs"]
mod tests;
