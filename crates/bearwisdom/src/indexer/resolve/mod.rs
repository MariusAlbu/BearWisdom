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
//   * engine     — SymbolIndex + chain walker + language resolver dispatch
//   * flow_emit  — FlowEmission data model
//   * reachability + synthesize_dispatch — post-resolution dead-code support
// =============================================================================

mod adapters;
pub mod engine;
pub mod flow_emit;
mod flow_pair;
mod indexes;
mod loop_body;
mod path_util;
pub mod reachability;
pub mod synthesize_dispatch;
mod write_buf;

use anyhow::{Context, Result};
use std::collections::HashMap;

use crate::db::Database;
use crate::indexer::project_context::ProjectContext;
use crate::types::ParsedFile;

use engine::ChainMiss;

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
// Compiler-resolve A/B gate
// ---------------------------------------------------------------------------

/// Process-level switch for compiler-style resolution routing.
///
/// When enabled, a coincidental same-name bind ("grep") on a ref whose head
/// is bound to a dependency by import/manifest is rejected and recorded as
/// external + demand instead of a false call-graph edge, and the eager
/// demand seed is skipped — the at-the-ref demand pull covers it. When
/// disabled, resolution behaves exactly as before. Read once and cached so
/// the parallel resolve loop pays a single `getenv` per process.
///
/// On for `BEARWISDOM_COMPILER_RESOLVE=1` or `=true`; off otherwise.
pub(crate) fn compiler_resolve_enabled() -> bool {
    use std::sync::OnceLock;
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("BEARWISDOM_COMPILER_RESOLVE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    })
}

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
    /// Chain walker bail-outs collected during this pass. The orchestrator
    /// (full.rs) feeds these into `expand_chain_reachability` to drive a
    /// second-pass `Ecosystem::resolve_symbol` reload.
    pub chain_misses: Vec<ChainMiss>,
}

impl ResolutionStats {
    /// `true` when the chain walker recorded no bail-outs — i.e. no external
    /// file demand was surfaced by this pass and the Stage 2 loop can stop.
    /// Used by the demand-driven pipeline as the fixpoint-exit condition.
    pub fn converged(&self) -> bool {
        self.chain_misses.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Resolve all references across all parsed files, writing edges,
/// unresolved refs, and external refs to the database. One-shot entry
/// point for callers that don't need iteration: runs `resolve_iteration`
/// once and then `finalize_resolution`.
///
/// Two-tier: language-specific resolvers first (1.0 confidence),
/// then heuristic fallback (0.50-0.95 confidence).
/// Unresolvable refs with a known external namespace go to `external_refs`;
/// truly unknown refs go to `unresolved_refs`.
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
/// `stats.converged()` reports whether the chain walker recorded any
/// bail-outs. Callers use that as the loop-exit signal.
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
    cached_index: &mut Option<engine::SymbolIndex>,
    new_files_slice: &[ParsedFile],
) -> Result<ResolutionStats> {
    resolve_iteration_with_cached_index_and_arena(
        db,
        parsed,
        symbol_id_map,
        project_ctx,
        cached_index,
        new_files_slice,
        std::sync::Arc::new(crate::type_checker::core::types::TypeArena::new()),
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
    cached_index: &mut Option<engine::SymbolIndex>,
    new_files_slice: &[ParsedFile],
    type_arena: std::sync::Arc<crate::type_checker::core::types::TypeArena>,
) -> Result<ResolutionStats> {
    if cached_index.is_none() {
        let mut index = engine::SymbolIndex::build_with_context_and_arena(
            parsed,
            symbol_id_map,
            project_ctx,
            type_arena,
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
    )
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
        conn.execute(
            "UPDATE symbols SET incoming_edge_count = COALESCE(
                (SELECT cnt FROM _edge_counts WHERE _edge_counts.id = symbols.id), 0)",
            [],
        )
        .context("Failed to materialize incoming_edge_count")?;
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
    reachability::materialize_reachability(db)
        .context("Failed to materialize reachability")?;
    crate::query::dead_code::materialize_package_resolution_health(db)
        .context("Failed to materialize package_resolution_health")?;

    Ok(())
}
