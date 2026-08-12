// =============================================================================
// indexer/full_resolve_phase.rs — plugin-state-aware resolve orchestration
//
// The full-index resolve step in three parts: materialize the externals the
// project's refs demand and build the Compilation tree; when that pull
// surfaced files no earlier plugin-state pass saw, refresh plugin cross-file
// state against the complete file universe and rebuild the tree if synthesis
// added members; then resolve every internal file against the (possibly
// rebuilt) tree. Split out of `full_index_inner` because the plugin-state
// refresh needs mutable `ProjectContext` access the resolve pass itself never
// does.
// =============================================================================

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::db::Database;
use crate::ecosystem::symbol_index::SymbolLocationIndex;
use crate::indexer::parse_file::with_resolve_pool;
use crate::indexer::plugin_state_phase;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::pipeline;
use crate::indexer::resolve::ResolutionStats;
use crate::indexer::write::SymbolIdMap;
use crate::languages::LanguageRegistry;
use crate::type_checker::core::types::TypeArena;
use crate::types::ParsedFile;

/// Materialize externals, refresh plugin cross-file state against any
/// demand-pulled batch, and resolve every internal file — the full-index
/// resolve step. `parsed` and `symbol_id_map` grow in place when the demand
/// pull surfaces files no earlier phase saw.
#[allow(clippy::too_many_arguments)]
pub fn resolve_with_plugin_refresh(
    db: &mut Database,
    parsed: &mut Vec<ParsedFile>,
    symbol_id_map: &mut SymbolIdMap,
    project_ctx: &mut ProjectContext,
    registry: &LanguageRegistry,
    project_root: &Path,
    arena: Arc<TypeArena>,
    loc: Arc<SymbolLocationIndex>,
) -> Result<ResolutionStats> {
    // Run the whole pass on the deep-stack resolve pool: building the
    // Compilation and walking chains over external `.d.ts` nests far past the
    // ~8 MB default stack (bundled/generated types — tRPC routers, recursive
    // mapped types).
    let (mut tree, ext_parsed, ext_id_map) = {
        let db_ref = &mut *db;
        let parsed_ref = &*parsed;
        let sid_ref = &*symbol_id_map;
        let pctx_ref = &*project_ctx;
        let arena_c = Arc::clone(&arena);
        let loc_c = Arc::clone(&loc);
        with_resolve_pool(move || {
            pipeline::materialize_and_build_tree(
                db_ref, parsed_ref, sid_ref, Some(pctx_ref), arena_c, loc_c,
            )
        })
        .context("Failed to materialize externals / build compilation")?
    };

    // A demand-pulled external (e.g. an Elixir test dependency's `__using__`
    // macro source, reached only because a project ref followed it during
    // this pass) never appeared in the eager `parsed` slice the caller's
    // pre-resolve plugin-state phases populated state and synthesized
    // members from — those files surface only here. Fold the pulled batch in
    // and re-run both phases so a plugin whose cross-file state depends on it
    // sees the complete file universe, then rebuild the tree if that
    // surfaced new member symbols the resolve pass below needs to see.
    if !ext_parsed.is_empty() {
        symbol_id_map.extend(ext_id_map);
        parsed.extend(ext_parsed);
        plugin_state_phase::populate_post_externals(
            registry, project_ctx, parsed, project_root, None,
        );
        let gained_members = plugin_state_phase::synthesize_and_persist(
            registry, project_ctx, parsed, db, symbol_id_map, arena.as_ref(),
        )?;
        if gained_members {
            let parsed_ref = &*parsed;
            let sid_ref = &*symbol_id_map;
            let pctx_ref = &*project_ctx;
            let arena_c = Arc::clone(&arena);
            tree = with_resolve_pool(move || {
                pipeline::rebuild_tree(parsed_ref, sid_ref, Some(pctx_ref), arena_c)
            });
        }
    }

    let db_ref = &mut *db;
    let parsed_ref = &*parsed;
    let sid_ref = &*symbol_id_map;
    let pctx_ref = &*project_ctx;
    with_resolve_pool(move || {
        pipeline::resolve_from_tree(db_ref, tree, parsed_ref, sid_ref, Some(pctx_ref))
    })
    .context("SemanticModel single-pass resolve failed")
}

#[cfg(test)]
#[path = "full_resolve_phase_tests.rs"]
mod tests;
