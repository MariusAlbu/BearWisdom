//! One import specifier → the module it names, from the importing file's
//! point of view: a declared provider, a relative path, a sibling workspace
//! package, a package entry, or a configured alias — in that order.
use super::{module_paths, ModuleId, ModuleInput};
use crate::indexer::resolve::engine::contract::SymbolLookup;
use rustc_hash::FxHashMap;

pub(super) fn link(
    paths: &FxHashMap<String, ModuleId>,
    input: &ModuleInput,
    spec: &str,
    lookup: &dyn SymbolLookup,
    providers: &FxHashMap<String, ModuleId>,
    redirects: &FxHashMap<ModuleId, ModuleId>,
) -> Option<ModuleId> {
    let redirect = |module| redirects.get(&module).copied().unwrap_or(module);
    if let Some(&module) = providers.get(spec) {
        return Some(module);
    }
    if let Some(base) = module_paths::relative_base(&input.path, spec) {
        return module_paths::find(&base, &input.paths, |path| paths.get(path).copied())
            .map(redirect);
    }
    // A sibling workspace package is its own source: its first declared entry
    // candidate an indexed file spells wins over any copy pulled from a
    // dependency directory.
    for base in lookup.workspace_package_entries(spec) {
        if let Some(module) =
            module_paths::find(&module_paths::normalize(base), &input.paths, |path| {
                paths.get(path).copied()
            })
        {
            return Some(redirect(module));
        }
    }
    // Only configured package entries/module aliases are candidates. The global
    // declaration-name and file-suffix indexes are not module evidence.
    if let Some(path) = lookup.resolve_module_from(&input.path, spec) {
        return paths
            .get(&module_paths::normalize(path))
            .copied()
            .map(redirect);
    }
    let alias = lookup.resolve_module_alias(lookup.package_id_for_file(&input.path), spec)?;
    module_paths::find(&module_paths::normalize(&alias), &input.paths, |path| {
        paths.get(path).copied()
    })
    .map(redirect)
}
