// =============================================================================
// engine/module_entry — module specifier → entry-file keys for the Compilation
//
// Populates the store's `module_entry` map, which `resolve_module_from`
// consults to link a bare module specifier to the indexed file its lookups
// start from. Two feeders per batch:
//   * package entries supplied by ecosystem virtual-path adapters
//     (barrel-preferring, depth-tie-broken),
//   * exact aliases supplied by ecosystem module-specifier adapters, and
//   * ambient-module declarations carried on `ParsedFile::declared_modules`
//     (`declare module 'virtual:x'`) — pure data consumption, so an internal
//     shim file keys its declared specifiers exactly like a package entry.
// =============================================================================

use rustc_hash::FxHashMap;

use crate::types::ParsedFile;

/// Insert this batch's module-entry keys. `reexport_map` must already hold
/// the batch's re-export rows (the barrel preference reads it).
pub(super) fn populate(
    module_entry: &mut FxHashMap<String, String>,
    reexport_map: &FxHashMap<String, Vec<(String, String)>>,
    parsed: &[ParsedFile],
) {
    // Bare package specifier → the indexed entry file. Ecosystem adapters
    // derive each external path's key; when a package contributes several
    // files prefer the barrel (non-empty re-exports), then the shallowest
    // path. Iterates a sorted list so the pick is reindex-deterministic.
    let mut paths: Vec<&str> = parsed.iter().map(|pf| pf.path.as_str()).collect();
    paths.sort_unstable();
    for path in paths {
        // Ecosystems may retain exact URI or package aliases beside the bare
        // package entry. First writer wins for deterministic reindexing.
        for alias in crate::ecosystem::module_specifier::entry_aliases(path) {
            module_entry
                .entry(alias)
                .or_insert_with(|| path.to_string());
        }

        let Some(pkg) = crate::ecosystem::module_specifier::package_entry_key(path) else {
            continue;
        };

        let has_reexports = reexport_map.get(path).is_some_and(|v| !v.is_empty());
        let depth = path.matches('/').count();
        let replace = match module_entry.get(&pkg) {
            None => true,
            Some(existing) => {
                let ex_has = reexport_map.get(existing).is_some_and(|v| !v.is_empty());
                let ex_depth = existing.matches('/').count();
                (has_reexports && !ex_has) || (has_reexports == ex_has && depth < ex_depth)
            }
        };
        if replace {
            module_entry.insert(pkg, path.to_string());
        }
    }

    // Declared ambient modules: language plugins turn declaration grammar into
    // exact aliases. The generic map only consumes those semantic aliases;
    // pattern spelling and admission belong to the plugin that parsed them.
    // First writer wins, and this pass runs after package entries, so an
    // augmentation never displaces a real package entry.
    for pf in parsed {
        let plugin = crate::languages::default_registry().get(&pf.language);
        for name in plugin.exact_module_entry_aliases(&pf.declared_modules) {
            module_entry.entry(name).or_insert_with(|| pf.path.clone());
        }
    }
}

#[cfg(test)]
#[path = "module_entry_tests.rs"]
mod tests;
