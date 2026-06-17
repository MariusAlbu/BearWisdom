// =============================================================================
// indexer/resolve/mod.rs — Reference resolution façade
//
// Single-tier resolution: every ref is either engine-resolved at
// `confidence = 1.0` (via `type_checker::core::DefaultResolver` plus per-
// language hook strategies), classified as external, or honestly
// unresolved. The pre-existing tier-2 heuristic fallback was deleted
// once every deterministic strategy it offered had been lifted into
// `DefaultResolver` and every language hook had been migrated to call
// it. See `type_checker/core/default_resolver.rs` for the strategy
// tower.
//
// This file is the public API. The implementation splits across siblings:
//
//   * loop_body  — the parallel per-file resolve loop
//   * write_buf  — per-worker write buffer + bulk SQL flush
//   * flow_pair  — Producer/Consumer pairing of FlowEmissions
//   * adapters   — framework-specific Consumer adapters (mailer / Next.js /
//                  extractor-emitted routes + DbSets)
//   * legacy     — SymbolIndex + chain walker + language resolver dispatch.
//                  Full reindex now routes through the `engine` island
//                  (SemanticModel); this path is reached only by INCREMENTAL
//                  reindex until the engine grows an incremental pass, then it
//                  is deleted.
//   * flow_emit  — FlowEmission data model
//   * reachability + synthesize_dispatch — post-resolution dead-code support
// =============================================================================

mod adapters;
pub mod flow_emit;
mod flow_pair;
mod indexes;
pub mod legacy;
mod loop_body;
mod path_util;
pub mod reachability;
mod return_inference;
pub mod engine;
pub mod synthesize_dispatch;
mod write_buf;

use anyhow::{Context, Result};
use std::collections::HashMap;

use crate::db::Database;
use crate::indexer::project_context::ProjectContext;
use crate::types::ParsedFile;

pub use adapters::append_db_route_consumer_emissions;
pub use flow_pair::flush_flow_emissions_public;
pub(crate) use loop_body::ResolveSideTables;
pub(crate) use write_buf::DeferredSpeculative;

#[cfg(test)]
pub(crate) use adapters::{
    extracted_db_sets_to_emissions, extracted_routes_to_emissions, mailer_template_name_for_path,
    nextjs_route_consumer_emissions, plugin_flow_emissions_to_emissions,
};
#[cfg(test)]
pub(crate) use flow_pair::_test_flush_flow_emissions;

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

// ---------------------------------------------------------------------------
// ResolutionStats — return value of every resolve_* entry point
// ---------------------------------------------------------------------------

/// Stats returned by `resolve_and_write` / `resolve_iteration`.
#[derive(Debug, Clone, Default)]
pub struct ResolutionStats {
    pub resolved: u64,
    pub engine_resolved: u64,
    pub unresolved: u64,
    pub external: u64,
    /// Function return types inferred from `return <expr>` sites this pass
    /// (qname → joined type), already conflict-filtered and limited to
    /// functions with no declared/known return. The orchestrator gap-fills
    /// these into the cached index and re-resolves so callers read the
    /// inferred return (INFER-3 / INFER-2).
    pub inferred_returns: std::collections::HashMap<String, String>,
    /// Internal files that can still gain an edge on a later pass — those with
    /// a remaining unresolved ref or an external ref. They form the shrinking
    /// frontier whose resolution can still change once externals are pulled;
    /// the full-index fixpoint feeds this back as the next pass's worklist so
    /// already-resolved files are skipped.
    pub frontier_files: Vec<String>,
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Resolve all references across all parsed files, writing edges,
/// unresolved refs, and external refs to the database. One-shot entry
/// point for callers that don't need iteration: runs `resolve_iteration`
/// once and then `finalize_resolution`.
///
/// Resolution is binary: a ref binds structurally at `RESOLVED_CONFIDENCE`
/// or stays unresolved. Unresolvable refs with a known external namespace go
/// to `external_refs`; truly unknown refs go to `unresolved_refs`.
pub fn resolve_and_write(
    db: &mut Database,
    parsed: &[ParsedFile],
    symbol_id_map: &HashMap<(String, String), i64>,
    project_ctx: Option<&ProjectContext>,
) -> Result<ResolutionStats> {
    let stats = loop_body::resolve_iteration_inner(db, parsed, symbol_id_map, project_ctx, false)?;
    finalize_resolution(db)?;
    Ok(stats)
}

/// Incremental variant: augments the SymbolIndex with all symbols from DB
/// so the engine resolver can find targets in unchanged files.
pub fn resolve_and_write_incremental(
    db: &mut Database,
    parsed: &[ParsedFile],
    symbol_id_map: &HashMap<(String, String), i64>,
    project_ctx: Option<&ProjectContext>,
) -> Result<ResolutionStats> {
    let stats = loop_body::resolve_iteration_inner(db, parsed, symbol_id_map, project_ctx, true)?;
    finalize_resolution(db)?;
    Ok(stats)
}

/// Same as `resolve_and_write_incremental` but threads a workspace
/// `TypeArena` through to `SymbolIndex::build_with_context_and_arena`.
pub fn resolve_and_write_incremental_and_arena(
    db: &mut Database,
    parsed: &[ParsedFile],
    symbol_id_map: &HashMap<(String, String), i64>,
    project_ctx: Option<&ProjectContext>,
    type_arena: std::sync::Arc<crate::type_checker::core::types::TypeArena>,
) -> Result<ResolutionStats> {
    let stats = loop_body::resolve_iteration_inner_with_arena(
        db,
        parsed,
        symbol_id_map,
        project_ctx,
        true,
        type_arena,
    )?;
    finalize_resolution(db)?;
    Ok(stats)
}

/// One resolution pass without post-processing. Writes edges / external_refs /
/// unresolved_refs the same way as `resolve_and_write` but leaves the
/// `incoming_edge_count` materialization to a later `finalize_resolution`
/// call — so the Stage 2 demand-driven pipeline can call this in a loop,
/// DELETE speculative unresolved/external rows between iterations, and
/// only finalize once the demand set reaches fixpoint.
///
/// `stats.frontier_files` lists the source files that recorded a chain miss —
/// the worklist the full-index fixpoint re-resolves on its next pass.
pub fn resolve_iteration(
    db: &mut Database,
    parsed: &[ParsedFile],
    symbol_id_map: &HashMap<(String, String), i64>,
    project_ctx: Option<&ProjectContext>,
) -> Result<ResolutionStats> {
    loop_body::resolve_iteration_inner(db, parsed, symbol_id_map, project_ctx, false)
}

/// Incremental iteration variant. Same shape as `resolve_iteration` but
/// augments the SymbolIndex with DB symbols first.
pub fn resolve_iteration_incremental(
    db: &mut Database,
    parsed: &[ParsedFile],
    symbol_id_map: &HashMap<(String, String), i64>,
    project_ctx: Option<&ProjectContext>,
) -> Result<ResolutionStats> {
    loop_body::resolve_iteration_inner(db, parsed, symbol_id_map, project_ctx, true)
}

/// Reuse-across-iterations variant. The orchestrator (full.rs) builds
/// the SymbolIndex once and threads it through expand-loop iterations
/// via `&mut Option<SymbolIndex>`. Each call:
///   - if `index` is `None`: builds via `build_with_context` (initial)
///   - if `index` is `Some`: reuses, augmenting with `new_files` if non-empty
///
/// On a 280k-symbol aspnetcore index the rebuild costs ~5-10s; running
/// it 8× across the expand loop is ~40-80s of redundant work this
/// avoids. Equivalent correctness for the resolved-edge counts.
pub fn resolve_iteration_with_cached_index(
    db: &mut Database,
    parsed: &[ParsedFile],
    symbol_id_map: &HashMap<(String, String), i64>,
    project_ctx: Option<&ProjectContext>,
    cached_index: &mut Option<legacy::SymbolIndex>,
    cached_engine: &mut Option<crate::type_checker::Engine<'static>>,
    cached_side_tables: &mut Option<loop_body::ResolveSideTables>,
    new_files_slice: &[ParsedFile],
    defer_speculative: Option<&mut DeferredSpeculative>,
    retry_files: Option<&std::collections::HashSet<String>>,
) -> Result<ResolutionStats> {
    resolve_iteration_with_cached_index_and_arena(
        db,
        parsed,
        symbol_id_map,
        project_ctx,
        cached_index,
        cached_engine,
        cached_side_tables,
        new_files_slice,
        std::sync::Arc::new(crate::type_checker::core::types::TypeArena::new()),
        defer_speculative,
        retry_files,
        std::sync::Arc::new(crate::ecosystem::symbol_index::SymbolLocationIndex::new()),
    )
}

/// Same as `resolve_iteration_with_cached_index` but threads a workspace
/// `TypeArena` through to `SymbolIndex::build_with_context_and_arena`. The
/// arena must be the same one the parse phase used so extractor-populated
/// TypeIds on `ExtractedSymbol` point into the same canonical table the
/// engine consults.
pub fn resolve_iteration_with_cached_index_and_arena(
    db: &mut Database,
    parsed: &[ParsedFile],
    symbol_id_map: &HashMap<(String, String), i64>,
    project_ctx: Option<&ProjectContext>,
    cached_index: &mut Option<legacy::SymbolIndex>,
    cached_engine: &mut Option<crate::type_checker::Engine<'static>>,
    cached_side_tables: &mut Option<loop_body::ResolveSideTables>,
    new_files_slice: &[ParsedFile],
    type_arena: std::sync::Arc<crate::type_checker::core::types::TypeArena>,
    defer_speculative: Option<&mut DeferredSpeculative>,
    retry_files: Option<&std::collections::HashSet<String>>,
    loc: std::sync::Arc<crate::ecosystem::symbol_index::SymbolLocationIndex>,
) -> Result<ResolutionStats> {
    if cached_index.is_none() {
        let mut index = legacy::SymbolIndex::build_with_context_and_arena(
            parsed,
            symbol_id_map,
            project_ctx,
            type_arena,
            loc,
        );
        let external_paths = loop_body::read_external_file_paths(db.conn());
        if !external_paths.is_empty() {
            index.set_external_paths(external_paths);
        }
        *cached_index = Some(index);
    } else if !new_files_slice.is_empty() {
        if let Some(idx) = cached_index.as_mut() {
            idx.augment_from_parsed(new_files_slice, symbol_id_map);
        }
    }
    loop_body::resolve_iteration_inner_with_index(
        db,
        parsed,
        symbol_id_map,
        project_ctx,
        cached_index.as_mut().expect("index is set above"),
        cached_engine,
        cached_side_tables,
        new_files_slice,
        defer_speculative,
        retry_files,
    )
}

/// Flush the speculative rows (unresolved + external refs) accumulated across
/// the demand-loop passes, once after the fixpoint settles. Clears the two
/// tables first so the result is exactly the final pass's set. Edges are already
/// durable (each pass flushes them via INSERT OR IGNORE), so this writes only
/// the speculative tables.
pub(crate) fn flush_deferred_speculative(
    db: &mut Database,
    deferred: &DeferredSpeculative,
) -> Result<()> {
    let conn = db.conn();
    let tx = conn
        .unchecked_transaction()
        .context("Failed to begin deferred speculative flush transaction")?;
    tx.execute("DELETE FROM unresolved_refs", [])
        .context("Failed to clear unresolved_refs before final flush")?;
    tx.execute("DELETE FROM external_refs", [])
        .context("Failed to clear external_refs before final flush")?;
    write_buf::flush_resolve_buf(&tx, deferred.buf(), true)
        .context("Failed to flush deferred speculative rows")?;
    tx.commit()
        .context("Failed to commit deferred speculative flush")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Post-resolution finalisation
// ---------------------------------------------------------------------------

/// Post-resolution DB maintenance. Runs once after the last call to
/// `resolve_iteration` in a demand-driven loop (or immediately after the
/// one-shot `resolve_and_write`). Rematerializes `incoming_edge_count` on
/// every symbol row so centrality / blast-radius queries stay O(1).
///
/// The join-update path (scan edges into a temp table, UPDATE once) is
/// O(E + S log D) — far cheaper than a correlated subquery that would
/// issue one COUNT per symbol row. Separated from `resolve_iteration` so
/// Stage 2 can call iteration multiple times without paying this cost on
/// every pass.
pub fn finalize_resolution(db: &mut Database) -> Result<()> {
    {
        let conn = db.conn();
        conn.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS _edge_counts (id INTEGER PRIMARY KEY, cnt INTEGER);
             DELETE FROM _edge_counts;
             INSERT INTO _edge_counts SELECT target_id, COUNT(*) FROM edges GROUP BY target_id;",
        )
        .context("Failed to build edge count temp table")?;
        // Materialize incoming_edge_count, touching only rows whose count
        // actually changed. The column is NOT NULL DEFAULT 0, so a symbol absent
        // from _edge_counts is already 0; the two targeted updates leave every
        // row with its correct count:
        //   (1) symbols with edges whose stored count is stale;
        conn.execute(
            "UPDATE symbols SET incoming_edge_count =
                (SELECT cnt FROM _edge_counts WHERE _edge_counts.id = symbols.id)
             WHERE id IN (SELECT id FROM _edge_counts)
               AND incoming_edge_count <>
                (SELECT cnt FROM _edge_counts WHERE _edge_counts.id = symbols.id)",
            [],
        )
        .context("Failed to update changed incoming_edge_count rows")?;
        //   (2) symbols that lost all incoming edges since the last finalize.
        conn.execute(
            "UPDATE symbols SET incoming_edge_count = 0
             WHERE incoming_edge_count <> 0
               AND id NOT IN (SELECT id FROM _edge_counts)",
            [],
        )
        .context("Failed to reset cleared incoming_edge_count rows")?;
        conn.execute("DELETE FROM _edge_counts", [])
            .context("Failed to clean up edge count temp table")?;
    }

    // L2+L1+L3 of the reachability-based dead-code stack:
    //   1. Synthesize dispatch_candidate edges so virtual-dispatch
    //      targets (trait/interface method → impl method) are present
    //      in the graph before the BFS runs.
    //   2. Rebuild the `entry_points` table from contributors.
    //   3. BFS from entry-point seeds through edges above the confidence
    //      threshold to materialize `reachability`.
    // Dead-code queries antijoin against `reachability` instead of
    // checking `incoming_edge_count = 0`.
    synthesize_dispatch::synthesize_dispatch_edges(db)
        .context("Failed to synthesize dispatch edges")?;
    crate::query::entry_points::rebuild_entry_points(db)
        .context("Failed to rebuild entry_points")?;
    reachability::materialize_reachability(db).context("Failed to materialize reachability")?;
    crate::query::dead_code::materialize_package_resolution_health(db)
        .context("Failed to materialize package_resolution_health")?;

    Ok(())
}
