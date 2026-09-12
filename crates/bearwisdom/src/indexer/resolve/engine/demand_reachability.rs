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

use crate::ecosystem::external_policy;
use crate::ecosystem::symbol_index::SymbolLocationIndex;
use crate::indexer::phase_timer;
use crate::type_checker::profile::language_profile::LanguageProfile;
use crate::types::ParsedFile;

use super::compilation::Compilation;
use super::demand_veto::{DemandVeto, FileLanguages};
use super::{externals_demand, type_mention_demand};

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
    augmentations: &mut Vec<crate::languages::ModuleAugmentation>,
) {
    let veto = DemandVeto::new(&pf.language, profiles, file_langs);
    {
        let _t = phase_timer::scope("demand.collect_refs");
        externals_demand::collect_external_files(&pf.refs, &veto, tree, loc, seen, next);
        type_mention_demand::collect_return_type_files(
            &pf.symbols,
            &pf.language,
            profiles,
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
        external_policy::collect_relative_supertypes(&pf.language, abs, &pf.refs, seen, next);
    }
    if let Some(profile) = profiles.get(pf.language.as_str()) {
        let _t = phase_timer::scope("demand.relative_hops");
        super::demand_relative_hops::collect(abs, &pf.language, profile, &pf.refs, seen, next);
    }
    // Per-language source-derived demand records stay with the file's active
    // plugin. The resolver reads source once and forwards normalized results.
    if let Ok(content) = std::fs::read_to_string(abs) {
        let plugin = crate::languages::default_registry().get(&pf.language);
        {
            let _t = phase_timer::scope("demand.decl_reachables");
            if let Some(dir) = abs.parent() {
                for spec in plugin.external_declaration_reachables(&pf.path, &content) {
                    if let Some(file) =
                        external_policy::resolve_relative_module(&pf.language, dir, &spec)
                    {
                        if seen.insert(file.clone()) {
                            next.push(file);
                        }
                    }
                }
            }
        }
        let _t = phase_timer::scope("demand.augmentations");
        augmentations.extend(plugin.external_module_augmentations(&content, &pf.path));
    }
}
