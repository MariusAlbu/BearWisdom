// =============================================================================
// indexer/plugin_state_phase.rs — run plugin cross-file state hooks at the
// right pipeline points.
//
// Three phases, each corresponding to one `LanguagePlugin` hook family, run
// in this order from `full.rs`:
//
//   1. `populate_pre_externals`  — `populate_project_state` over project
//      files only, before external dependencies are walked.
//   2. `populate_post_externals` — `populate_project_state_post_externals`
//      over the project+externals merged slice, once external symbols exist
//      for a plugin's cross-file map to bind against.
//   3. `synthesize_and_persist`  — `synthesize_project_symbols`, run after
//      (2) so a plugin's flattened cross-file state (e.g. Elixir's `use`-
//      injection map) is final. A macro-injection construct can give a
//      module real members with no textual trace in that module's own
//      source; each affected file's original symbols were already written
//      by the streaming per-file pass earlier in `full.rs`, so the addition
//      is spliced into that file's `ParsedFile::symbols` and the one file is
//      re-persisted (DELETE+reinsert — safe, since `refs`/`routes` are
//      unchanged and only `symbols` gains rows a hand-written declaration
//      didn't already cover).
// =============================================================================

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use tracing::info;

use crate::db::Database;
use crate::indexer::plugin_state::PluginStateBag;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::write::{self, SymbolIdMap};
use crate::languages::LanguageRegistry;
use crate::languages::robot::RobotExternalSources;
use crate::type_checker::core::types::TypeArena;
use crate::types::{ExtractedSymbol, ParsedFile};

/// Phase 1: each active plugin scans the parsed file slice and stores its
/// cross-file state in the bag. Populate into a separate bag first to
/// satisfy the borrow checker (can't borrow `project_ctx` mutably and
/// immutably at the same time), then assign the completed bag.
pub fn populate_pre_externals(
    registry: &LanguageRegistry,
    project_ctx: &mut ProjectContext,
    parsed: &[ParsedFile],
    project_root: &Path,
) {
    let mut plugin_state = PluginStateBag::new();
    for plugin in registry.all() {
        if !project_ctx.language_presence.contains(plugin.id()) {
            continue;
        }
        plugin.populate_project_state(&mut plugin_state, parsed, project_root, project_ctx);
    }
    project_ctx.plugin_state = plugin_state;
}

/// Phase 2: `populate_project_state` ran on project files only (before
/// externals). Plugins whose binding maps must see externally-walked
/// symbols override `populate_project_state_post_externals` to rebuild
/// against the merged slice; the rest keep their pre-externals entry
/// untouched. Robot is the first user: it re-resolves `Library
/// SeleniumLibrary` to the site-packages package + its DynamicCore keyword
/// methods now that those files are in `parsed`.
///
/// `robot_external_sources` is stashed in the bag first so the rebuild can
/// read the `ext:` library files' on-disk source for the keyword-method
/// scan — the hook reads it back from the same bag it writes into. The bag
/// is then taken out so a hook can hold `&mut bag` while `project_ctx` is
/// still borrowed immutably for the same call.
pub fn populate_post_externals(
    registry: &LanguageRegistry,
    project_ctx: &mut ProjectContext,
    parsed: &[ParsedFile],
    project_root: &Path,
    robot_external_sources: RobotExternalSources,
) {
    project_ctx.plugin_state.set::<RobotExternalSources>(robot_external_sources);
    let mut bag = std::mem::take(&mut project_ctx.plugin_state);
    for plugin in registry.all() {
        if !project_ctx.language_presence.contains(plugin.id()) {
            continue;
        }
        plugin.populate_project_state_post_externals(&mut bag, parsed, project_root, project_ctx);
    }
    project_ctx.plugin_state = bag;
}

/// Phase 3: collect every plugin's synthesized `(path, symbols)` pairs,
/// splice each into `parsed[idx].symbols`, then re-persist just that one
/// file via the existing incremental writer (DELETE-then-reinsert of that
/// file's symbols/imports — a hand-written declaration always wins, since
/// `LanguagePlugin::synthesize_project_symbols` implementations drop any
/// synthesized symbol whose `qualified_name` collides with a real one
/// before returning it).
pub fn synthesize_and_persist(
    registry: &LanguageRegistry,
    project_ctx: &ProjectContext,
    parsed: &mut [ParsedFile],
    db: &mut Database,
    symbol_id_map: &mut SymbolIdMap,
    workspace_arena: &TypeArena,
) -> Result<()> {
    let mut synthesized_by_path: HashMap<String, Vec<ExtractedSymbol>> = HashMap::new();
    for plugin in registry.all() {
        if !project_ctx.language_presence.contains(plugin.id()) {
            continue;
        }
        for (path, syms) in plugin.synthesize_project_symbols(&project_ctx.plugin_state, parsed) {
            synthesized_by_path.entry(path).or_default().extend(syms);
        }
    }
    if synthesized_by_path.is_empty() {
        return Ok(());
    }

    let path_to_idx: HashMap<String, usize> = parsed
        .iter()
        .enumerate()
        .map(|(i, pf)| (pf.path.clone(), i))
        .collect();
    let mut synthesized_count = 0u32;
    for (path, new_syms) in synthesized_by_path {
        let Some(&idx) = path_to_idx.get(&path) else {
            continue;
        };
        synthesized_count += new_syms.len() as u32;
        parsed[idx].symbols.extend(new_syms);
        let origin = if parsed[idx].path.starts_with("ext:") {
            "external"
        } else {
            "internal"
        };
        let (_file_map, sym_map) = write::write_parsed_files_with_origin_incremental(
            db,
            std::slice::from_ref(&parsed[idx]),
            origin,
            Some(workspace_arena),
        )
        .with_context(|| format!("Failed to persist synthesized symbols for {path}"))?;
        symbol_id_map.extend(sym_map);
    }
    if synthesized_count > 0 {
        info!("Synthesized {synthesized_count} project-wide member symbols");
    }
    Ok(())
}
