// =============================================================================
// engine/module_entry — module specifier → entry-file keys for the Compilation
//
// Populates the store's `module_entry` map, which `resolve_module_from`
// consults to link a bare module specifier to the indexed file its lookups
// start from. Two feeders per batch:
//   * package entries derived from the shared `ext:<lang>:<pkg>/…` path
//     convention (barrel-preferring, depth-tie-broken), and
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
    // Bare package specifier → the indexed entry file. For each external
    // `ext:<lang>:<pkg>/…` file key `<pkg>` (scoped packages keep both
    // leading segments); when a package contributes several files prefer the
    // barrel (non-empty re-exports), then the shallowest path. Iterates a
    // sorted list so the pick is reindex-deterministic.
    let mut ext_paths: Vec<&str> = parsed
        .iter()
        .map(|pf| pf.path.as_str())
        .filter(|p| p.starts_with("ext:"))
        .collect();
    ext_paths.sort_unstable();
    for path in ext_paths {
        let Some(pkg) = package_from_ext_path(path) else {
            continue;
        };
        let has_reexports = reexport_map.get(path).is_some_and(|v| !v.is_empty());
        let depth = path.matches('/').count();
        let replace = match module_entry.get(pkg) {
            None => true,
            Some(existing) => {
                let ex_has = reexport_map.get(existing).is_some_and(|v| !v.is_empty());
                let ex_depth = existing.matches('/').count();
                (has_reexports && !ex_has) || (has_reexports == ex_has && depth < ex_depth)
            }
        };
        if replace {
            module_entry.insert(pkg.to_string(), path.to_string());
        }
    }

    // Declared ambient modules: each `declare module '<name>'` literal keys
    // the declaring file, so an import of that specifier links to it. A `*`
    // pattern (`declare module '*.css'`) is a wildcard shape, not an exact
    // specifier, and gets no key. First writer wins, and the pass runs after
    // the package-entry pass, so a module augmentation (`declare module
    // 'vue'`) never displaces a real package entry.
    for pf in parsed {
        for name in &pf.declared_modules {
            if name.contains('*') {
                continue;
            }
            module_entry
                .entry(name.clone())
                .or_insert_with(|| pf.path.clone());
        }
    }
}

/// The package key of an `ext:<lang>:<pkg>/…` virtual path — scoped packages
/// keep both leading segments (`@scope/name`).
fn package_from_ext_path(path: &str) -> Option<&str> {
    let after_lang = path.strip_prefix("ext:")?.split_once(':')?.1;
    if after_lang.starts_with('@') {
        let mut segs = after_lang.splitn(3, '/');
        let scope = segs.next()?;
        let name = segs.next()?;
        Some(&after_lang[..scope.len() + 1 + name.len()])
    } else {
        after_lang.split('/').next()
    }
}

#[cfg(test)]
#[path = "module_entry_tests.rs"]
mod tests;
