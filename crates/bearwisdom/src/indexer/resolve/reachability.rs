// =============================================================================
// indexer/resolve/reachability.rs — Materialize the reachability table (L3)
//
// BFS from every row in `entry_points` outward through `edges` whose
// confidence clears `REACHABILITY_CONFIDENCE_THRESHOLD`. For each reached
// symbol, record:
//
//   • min_distance     — BFS layer (hop count from nearest entry point)
//   • path_confidence  — min edge confidence along the best chosen path
//   • via_kind         — kind of the edge that delivered this symbol on
//                        the chosen path (helps surface "alive only via
//                        synthesized dispatch" cases in Phase 4)
//
// Dead-code queries antijoin against this table: a symbol absent from
// `reachability` is unreachable from any entry point and is the canonical
// dead-code signal.
//
// Algorithm: layer-by-layer BFS. Within a layer, when two paths reach the
// same node we keep the one with the highest path_confidence. This is
// O(V + E) — single pass over the adjacency list per layer.
// =============================================================================

use crate::db::Database;
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};

/// Edges below this confidence are ignored by the BFS. Phase 7 will make
/// this configurable; for v1 it's a constant — 0.5 splits "first-class
/// resolver edge" from "best-guess heuristic" cleanly across every
/// language's existing strategy mix.
pub const REACHABILITY_CONFIDENCE_THRESHOLD: f64 = 0.5;

/// (Re)build the `reachability` table from the current `entry_points` +
/// `edges` state. Idempotent — clears the table and rewrites it.
///
/// Called from `finalize_resolution` after `incoming_edge_count` is up
/// to date. Tests that bypass the resolver call it directly via the
/// lazy path inside `query::dead_code::find_dead_code`.
pub fn materialize_reachability(db: &Database) -> Result<()> {
    let conn = db.conn();

    // --- Load entry points (BFS seed set) ------------------------------------
    // Filters out `kind = 'test'` rows — test functions are tracked in
    // `entry_points` so the `find_entry_points` report can surface them,
    // but using them as reachability anchors would make every helper they
    // call appear alive even when production code never touches it. The
    // dead-code query still excludes test FILES via `is_test_file()`
    // independent of reachability.
    let mut seeds: Vec<i64> = Vec::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT symbol_id FROM entry_points \
                 WHERE kind != 'test'",
            )
            .context("reachability: prepare entry_points scan")?;
        for row in stmt
            .query_map([], |r| r.get::<_, i64>(0))
            .context("reachability: execute entry_points scan")?
            .flatten()
        {
            seeds.push(row);
        }
    }

    // --- Load adjacency: source_id -> Vec<(target_id, confidence, kind)> -----
    // Single scan of `edges`, filtered by the confidence floor up-front so
    // weakly-resolved edges don't bloat the in-memory graph.
    let mut adj: HashMap<i64, Vec<(i64, f64, String)>> = HashMap::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT source_id, target_id, confidence, kind \
                 FROM edges \
                 WHERE confidence >= ?1",
            )
            .context("reachability: prepare edges scan")?;
        let rows = stmt
            .query_map([REACHABILITY_CONFIDENCE_THRESHOLD], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, f64>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .context("reachability: execute edges scan")?;
        for row in rows.flatten() {
            let (src, tgt, conf, kind) = row;
            adj.entry(src).or_default().push((tgt, conf, kind));
        }
    }

    // --- BFS -----------------------------------------------------------------
    // Per-node state: (min_distance, best path_confidence among shortest
    // paths, via_kind for that path). Entry points are at distance 0 with
    // confidence 1.0 and no incoming edge.
    let mut dist: HashMap<i64, u32> = HashMap::with_capacity(seeds.len());
    let mut conf: HashMap<i64, f64> = HashMap::with_capacity(seeds.len());
    let mut via: HashMap<i64, Option<String>> = HashMap::with_capacity(seeds.len());
    let mut frontier: HashSet<i64> = HashSet::with_capacity(seeds.len());

    for ep in &seeds {
        if dist.insert(*ep, 0).is_none() {
            conf.insert(*ep, 1.0);
            via.insert(*ep, None);
            frontier.insert(*ep);
        }
    }

    let mut depth: u32 = 0;
    while !frontier.is_empty() {
        let next_depth = depth + 1;
        // (target_id) -> (best path_confidence at this layer, via_kind)
        let mut next_layer: HashMap<i64, (f64, String)> = HashMap::new();

        for src in &frontier {
            let src_conf = *conf.get(src).unwrap_or(&1.0);
            let Some(neighbors) = adj.get(src) else {
                continue;
            };
            for (tgt, edge_conf, edge_kind) in neighbors {
                // Skip already-reached at strictly shallower depth — the
                // BFS guarantee says shortest-distance won't improve. If
                // the target is at THIS depth via another path, keep the
                // better confidence; otherwise leave it alone.
                if let Some(&existing) = dist.get(tgt) {
                    if existing < next_depth {
                        continue;
                    }
                    if existing == next_depth {
                        let new_conf = src_conf.min(*edge_conf);
                        let cur = *conf.get(tgt).unwrap_or(&0.0);
                        if new_conf > cur {
                            conf.insert(*tgt, new_conf);
                            via.insert(*tgt, Some(edge_kind.clone()));
                        }
                        continue;
                    }
                    // existing > next_depth is impossible in BFS — earlier
                    // layers always commit before later ones — but fall
                    // through defensively so future refactors don't break
                    // monotonically.
                }
                let new_conf = src_conf.min(*edge_conf);
                match next_layer.get(tgt) {
                    Some((existing, _)) if *existing >= new_conf => {}
                    _ => {
                        next_layer.insert(*tgt, (new_conf, edge_kind.clone()));
                    }
                }
            }
        }

        if next_layer.is_empty() {
            break;
        }

        frontier.clear();
        for (tgt, (c, k)) in next_layer {
            dist.insert(tgt, next_depth);
            conf.insert(tgt, c);
            via.insert(tgt, Some(k));
            frontier.insert(tgt);
        }
        depth = next_depth;
    }

    // --- Persist -------------------------------------------------------------
    // Wrap in a transaction so query-side readers never observe a partial
    // rewrite. `unchecked_transaction` is the right primitive here:
    // `Database::conn()` returns a shared ref and the writer lock is
    // single-threaded during `finalize_resolution`, so we can wrap a
    // transaction around the DELETE + N×INSERT without contention.
    let tx = conn
        .unchecked_transaction()
        .context("reachability: begin transaction")?;
    tx.execute("DELETE FROM reachability", [])
        .context("reachability: clear table")?;
    {
        let mut stmt = tx
            .prepare(
                "INSERT INTO reachability \
                 (symbol_id, min_distance, path_confidence, via_kind) \
                 VALUES (?1, ?2, ?3, ?4)",
            )
            .context("reachability: prepare insert")?;
        for (sid, d) in &dist {
            let c = *conf.get(sid).unwrap_or(&1.0);
            let v = via.get(sid).cloned().flatten();
            stmt.execute(rusqlite::params![sid, d, c, v])
                .context("reachability: insert row")?;
        }
    }
    tx.commit().context("reachability: commit")?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "reachability_tests.rs"]
mod tests;
