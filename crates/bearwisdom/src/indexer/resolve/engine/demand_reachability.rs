// =============================================================================
// engine/demand_reachability — next-frontier collection from one external file
//
// Given one just-materialized external file, collect every further external
// file its API surface demands — ref targets, return/callback-param type
// mentions, relative supertype imports, plugin-declared reachables, and TS
// module augmentations — into the closure's `seen`/`next` frontier. Each
// collection stage runs under a phase-timer scope so a slow demand closure
// localizes to the stage responsible.
// =============================================================================

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use rustc_hash::FxHashMap;

use crate::ecosystem::symbol_index::SymbolLocationIndex;
use crate::indexer::phase_timer;
use crate::type_checker::profile::language_profile::LanguageProfile;
use crate::types::ParsedFile;

use super::compilation::Compilation;
use super::demand_veto::{DemandVeto, FileLanguages};
use super::{externals_demand, module_augmentation, relative_imports, type_mention_demand};

/// Collect the demand frontier one materialized external file exposes.
#[allow(clippy::too_many_arguments)]
pub(super) fn collect(
    abs: &Path,
    pf: &ParsedFile,
    profiles: &FxHashMap<&'static str, &'static LanguageProfile>,
    file_langs: &FileLanguages<'_>,
    tree: &Compilation,
    loc: &SymbolLocationIndex,
    seen: &mut HashSet<PathBuf>,
    next: &mut Vec<PathBuf>,
    augmentations: &mut Vec<(String, String, String)>,
) {
    let veto = DemandVeto::new(&pf.language, profiles, file_langs);
    {
        let _t = phase_timer::scope("demand.collect_refs");
        externals_demand::collect_external_files(&pf.refs, &veto, tree, loc, seen, next);
        type_mention_demand::collect_return_type_files(
            &pf.symbols,
            &pf.language,
            tree,
            loc,
            seen,
            next,
        );
        type_mention_demand::collect_callback_param_type_files(
            &pf.symbols,
            &pf.language,
            profiles,
            tree,
            loc,
            seen,
            next,
        );
        relative_imports::collect_relative_supertype_imports(abs, &pf.refs, seen, next);
    }
    // Per-language extra reachability (e.g. Angular NgModule → component
    // .d.ts) — dispatched to the file's plugin so framework specifics stay
    // out of the generic resolve pipeline.
    {
        let _t = phase_timer::scope("demand.decl_reachables");
        if let Ok(content) = std::fs::read_to_string(abs) {
            let plugin = crate::languages::default_registry().get(&pf.language);
            if let Some(dir) = abs.parent() {
                for spec in plugin.external_declaration_reachables(&pf.path, &content) {
                    if let Some(file) = relative_imports::resolve_relative_ts_module(dir, &spec) {
                        if seen.insert(file.clone()) {
                            next.push(file);
                        }
                    }
                }
            }
        }
    }
    let _t = phase_timer::scope("demand.aug_scan");
    module_augmentation::collect_module_augmentations(abs, &pf.path, augmentations);
}
