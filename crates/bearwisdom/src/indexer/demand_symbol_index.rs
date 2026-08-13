// =============================================================================
// indexer/demand_symbol_index.rs — assemble the demand-driven symbol index
//
// One call per demand-driven ecosystem, merged into a process-wide master
// index that Stage 2's pull loop queries to locate files on demand.
// =============================================================================

use std::collections::HashMap;
use std::sync::Arc;

use tracing::info;

use crate::ecosystem::externals::ExternalDepRoot;
use crate::ecosystem::{Ecosystem, SymbolLocationIndex};

/// Drop every registered ecosystem's process-lifetime demand caches, so a
/// prior run's demand set neither pins memory into this run nor influences
/// its parse results. Incremental indexing never re-enters the externals
/// stage, so watch-mode events keep caches warm.
fn reset_all_demand_caches() {
    for eco in crate::ecosystem::default_registry().all() {
        eco.reset_demand_caches();
    }
}

/// Build the master demand-driven symbol index from every ecosystem that
/// opted into demand-driven parsing. This call marks the index-run boundary:
/// it first drops every registered ecosystem's process-lifetime demand
/// caches via `reset_all_demand_caches`, so each run resolves from a cold
/// state. Ecosystem tags are sorted before
/// iterating: `demand_driven_by_eco` is a HashMap, whose iteration order is
/// randomized per process, and `SymbolLocationIndex::extend` is first-writer-
/// wins on the `(module, name)` axis — an unsorted iteration would let a
/// cross-ecosystem key collision resolve to a different winner each run.
pub(crate) fn build_demand_symbol_index(
    demand_driven_by_eco: &HashMap<&'static str, Vec<ExternalDepRoot>>,
    demand_driven_ecosystems: &HashMap<&'static str, Arc<dyn Ecosystem>>,
) -> SymbolLocationIndex {
    reset_all_demand_caches();
    let mut symbol_index = SymbolLocationIndex::new();
    let _t_symidx = Some(crate::indexer::phase_timer::scope("externals.build_symbol_index"));
    let mut eco_tags: Vec<&'static str> = demand_driven_by_eco.keys().copied().collect();
    eco_tags.sort_unstable();
    for tag in &eco_tags {
        let roots = &demand_driven_by_eco[tag];
        let Some(eco) = demand_driven_ecosystems.get(tag) else {
            continue;
        };
        let mut idx = {
            let _t = crate::indexer::phase_timer::scope("externals.build_symbol_index.per_eco");
            eco.build_symbol_index(roots)
        };
        // A single-language ecosystem's dep roots hold files no other
        // language ever produces — stamp every path with that language so a
        // pulled file's extractor is chosen from its OWNING ecosystem rather
        // than re-derived from an extension another language family also
        // claims (FPC `.pp` units vs Puppet `.pp` manifests). Multi-language
        // ecosystems (Maven, npm, Hex) are left untagged — per-file
        // extension/import detection is the correct dispatch for those.
        let langs = eco.languages();
        if langs.len() == 1 {
            idx.tag_language(langs[0]);
        }
        if !idx.is_empty() {
            info!(
                "Built demand-driven symbol index for {}: {} entries across {} roots",
                tag,
                idx.len(),
                roots.len()
            );
        }
        symbol_index.extend(idx);
    }
    symbol_index
}

#[cfg(test)]
#[path = "demand_symbol_index_tests.rs"]
mod tests;
