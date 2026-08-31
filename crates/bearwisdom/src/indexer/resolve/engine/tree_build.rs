// =============================================================================
// engine/tree_build.rs — Compilation construction, split from the resolve pass
//
// Building the tree and resolving refs against it are two different
// responsibilities: construction needs only shared `ProjectContext` access,
// while a plugin-state refresh between the two (see
// `indexer::plugin_state_phase`) needs mutable access the resolve pass itself
// never does. Carved out of `pipeline.rs` so `full_index` can run that refresh
// between `materialize_and_build_tree` and `pipeline::resolve_from_tree`.
// =============================================================================

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::db::Database;
use crate::ecosystem::symbol_index::SymbolLocationIndex;
use crate::indexer::write::SymbolIds;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::compilation::Compilation;
use crate::indexer::resolve::engine::externals_demand::materialize_externals;
use crate::indexer::resolve::engine::file_context::build_profiles;
use crate::type_checker::core::types::TypeArena;
use crate::types::ParsedFile;

/// Build the `Compilation` from `parsed` and grow it with the externals the
/// project's refs demand. Returns the tree plus the demand-pulled external
/// batch and its DB id map — `parsed` never included them (they surface only
/// during this call), so a caller whose plugins carry `parsed`-derived
/// cross-file state (Elixir's `use`-injection map) needs both to refresh that
/// state before resolving. Split out of `resolve_single_pass` so the caller
/// can run that refresh, with mutable `ProjectContext` access, between
/// materialization and `pipeline::resolve_from_tree` — this function only
/// needs shared access, matching `Compilation::build_with_context`.
pub fn materialize_and_build_tree(
    db: &mut Database,
    parsed: &[ParsedFile],
    symbol_id_map: &SymbolIds,
    project_ctx: Option<&ProjectContext>,
    arena: Arc<TypeArena>,
    loc: Arc<SymbolLocationIndex>,
) -> Result<(Compilation, Vec<ParsedFile>, SymbolIds)> {
    let ambient_qnames = crate::ecosystem::ambient::ambient_global_qnames(parsed);
    let mut tree = Compilation::build_with_context(
        parsed,
        symbol_id_map,
        Arc::clone(&arena),
        project_ctx,
        &ambient_qnames,
    );
    let profiles = build_profiles();
    let (ext_parsed, ext_id_map) =
        materialize_externals(db, &mut tree, parsed, &loc, &arena, &profiles)
            .context("Failed to materialize external symbols")?;
    Ok((tree, ext_parsed, ext_id_map))
}

/// Rebuild the `Compilation` from `parsed` and the (now-complete)
/// `symbol_id_map`, without re-running externals materialization. Used after
/// a post-materialization plugin-state refresh splices newly-synthesized
/// member symbols into `parsed` (e.g. Elixir `__using__` `Def` facts that
/// only became visible once a demand-pulled dependency was in view) so the
/// tree the resolve pass walks includes them.
pub fn rebuild_tree(
    parsed: &[ParsedFile],
    symbol_id_map: &SymbolIds,
    project_ctx: Option<&ProjectContext>,
    arena: Arc<TypeArena>,
) -> Compilation {
    let ambient_qnames = crate::ecosystem::ambient::ambient_global_qnames(parsed);
    Compilation::build_with_context(parsed, symbol_id_map, arena, project_ctx, &ambient_qnames)
}
