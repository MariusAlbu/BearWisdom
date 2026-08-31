// =============================================================================
// indexer/resolve/mod.rs — Reference resolution façade
//
// Single-tier resolution: every ref is either engine-resolved at
// `confidence = 1.0` via the rule-based `SemanticModel` and chain walker,
// classified as external, or honestly unresolved. The full-index path runs
// `indexer::full_resolve_phase::resolve_with_plugin_refresh` (materialize +
// build via `engine::pipeline::materialize_and_build_tree`, a plugin-state
// refresh, then `engine::pipeline::resolve_from_tree`); the incremental path
// runs `engine::pipeline::resolve_incremental_pass`.
//
// This file is the public API. The implementation splits across siblings:
//
//   * engine          — rule-based SemanticModel + Compilation
//   * flow_pair       — Producer/Consumer pairing of FlowEmissions
//   * adapters        — framework-specific Consumer adapters (mailer / Next.js /
//                       extractor-emitted routes + DbSets)
//   * flow_emit       — FlowEmission data model
//   * reachability + synthesize_dispatch — post-resolution dead-code support
// =============================================================================

mod adapters;
pub mod flow_emit;
mod flow_pair;
pub mod reachability;
pub mod engine;
pub mod synthesize_dispatch;

use anyhow::{Context, Result};
use std::collections::HashMap;

use crate::db::Database;
use crate::indexer::write::SymbolIds;
use crate::indexer::project_context::ProjectContext;
use crate::types::ParsedFile;

pub use adapters::append_db_route_consumer_emissions;
pub use flow_pair::flush_flow_emissions_public;

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

/// Stats returned by resolve entry points.
#[derive(Debug, Clone, Default)]
pub struct ResolutionStats {
    pub resolved: u64,
    pub engine_resolved: u64,
    pub unresolved: u64,
    pub external: u64,
    /// Function return types inferred from `return <expr>` sites this pass
    /// (qname → joined type), already conflict-filtered and limited to
    /// functions with no declared/known return.
    pub inferred_returns: std::collections::HashMap<String, String>,
    /// Internal files that can still gain an edge on a later pass.
    pub frontier_files: Vec<String>,
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Incremental variant: resolves the changed files through the engine, loading
/// the unchanged remainder (and previously-materialized externals) from the DB
/// via `Compilation::ingest_from_db`.
pub fn resolve_and_write_incremental(
    db: &mut Database,
    parsed: &[ParsedFile],
    symbol_id_map: &SymbolIds,
    project_ctx: Option<&ProjectContext>,
) -> Result<ResolutionStats> {
    let arena = std::sync::Arc::new(crate::type_checker::core::types::TypeArena::new());
    // Restore the prior build's arena before resolution interns the changed
    // batch, so persisted `field_type_id` / `return_type_id` indices stay valid.
    if let Ok(blob) = db.conn().query_row(
        "SELECT value FROM _bearwisdom_meta WHERE key = 'type_arena_snapshot'",
        [],
        |r| r.get::<_, String>(0),
    ) {
        arena.restore_snapshot(&blob);
    }
    let stats =
        engine::pipeline::resolve_incremental_pass(db, parsed, symbol_id_map, project_ctx, arena)?;
    finalize_resolution(db)?;
    Ok(stats)
}

/// Same as `resolve_and_write_incremental` but threads the workspace `TypeArena`
/// the parse phase used, so extractor-set TypeIds align with the engine's.
pub fn resolve_and_write_incremental_and_arena(
    db: &mut Database,
    parsed: &[ParsedFile],
    symbol_id_map: &SymbolIds,
    project_ctx: Option<&ProjectContext>,
    type_arena: std::sync::Arc<crate::type_checker::core::types::TypeArena>,
) -> Result<ResolutionStats> {
    let stats = engine::pipeline::resolve_incremental_pass(
        db,
        parsed,
        symbol_id_map,
        project_ctx,
        type_arena,
    )?;
    finalize_resolution(db)?;
    Ok(stats)
}

// ---------------------------------------------------------------------------
// Post-resolution finalisation
// ---------------------------------------------------------------------------

/// Post-resolution DB maintenance. Runs once after a resolve pass.
/// Rematerializes `incoming_edge_count` on every symbol row so centrality /
/// blast-radius queries stay O(1).
///
/// The join-update path (scan edges into a temp table, UPDATE once) is
/// O(E + S log D) — far cheaper than a correlated subquery that would
/// issue one COUNT per symbol row.
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
    synthesize_dispatch::synthesize_dispatch_edges(db)
        .context("Failed to synthesize dispatch edges")?;
    crate::query::entry_points::rebuild_entry_points(db)
        .context("Failed to rebuild entry_points")?;
    reachability::materialize_reachability(db).context("Failed to materialize reachability")?;
    crate::query::dead_code::materialize_package_resolution_health(db)
        .context("Failed to materialize package_resolution_health")?;

    Ok(())
}
