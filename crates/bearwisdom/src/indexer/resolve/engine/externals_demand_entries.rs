// =============================================================================
// engine/externals_demand_entries — what the location index tells the
// compilation once its files are ingested
//
// The ecosystem that walked a package knows its published entry file and its
// cross-package re-export bridges. Both are handed to the compilation after
// the demand batch is ingested, keyed by the virtual paths the batch carries.
// =============================================================================

use std::collections::HashSet;

use crate::ecosystem::symbol_index::SymbolLocationIndex;
use crate::indexer::resolve::engine::compilation::Compilation;
use crate::indexer::symbol_ids::SymbolIds;
use crate::types::ParsedFile;

use super::externals_demand::{language_from_file_ext, virtual_path_for_indexed_file};

/// The package entries the ecosystem published for files this batch pulled.
/// An ecosystem-declared entry outranks the compilation's own barrel/depth
/// pick, which cannot see a manifest.
pub(super) fn apply_package_entries(
    tree: &mut Compilation,
    loc: &SymbolLocationIndex,
    ext_parsed: &[ParsedFile],
    ids: &SymbolIds,
) {
    let pulled: HashSet<&str> = ext_parsed.iter().map(|pf| pf.path.as_str()).collect();
    let entries: Vec<(String, String)> = loc
        .module_entries()
        .filter_map(|(module, file)| {
            let lang = language_from_file_ext(file)?;
            let vpath = virtual_path_for_indexed_file(file, lang);
            pulled
                .contains(vpath.as_str())
                .then(|| (module.to_string(), vpath))
        })
        .collect();
    if !entries.is_empty() {
        tree.apply_package_entries(&entries, ext_parsed, ids);
    }
}

/// Cross-package re-export aliases: `{importing_module}.{name}` resolves to
/// the sibling package's declaration now that both sides are ingested. The
/// alias qname is assembled here; the target qname derives from the
/// declaring file's virtual-path package prefix — the same prefix its
/// materialized symbols carry.
pub(super) fn apply_reexport_aliases(tree: &mut Compilation, loc: &SymbolLocationIndex) {
    let aliases: Vec<(String, String, String)> = loc
        .reexport_aliases()
        .filter_map(|(module, name, target_file, target_name)| {
            let lang = language_from_file_ext(target_file)?;
            let vpath = virtual_path_for_indexed_file(target_file, lang);
            let target_qname = crate::languages::default_registry()
                .get(lang)
                .external_reexport_target_qname(&vpath, target_name)?;
            Some((
                crate::indexer::resolve::engine::support::join_index_qname(module, name),
                target_qname,
                vpath,
            ))
        })
        .collect();
    if !aliases.is_empty() {
        tree.apply_external_reexport_aliases(&aliases);
    }
}
