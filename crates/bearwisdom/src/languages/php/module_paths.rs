// =============================================================================
// languages/php/module_paths — PHP namespace spelling to physical files
// =============================================================================

use crate::indexer::resolve::engine::support::path_contains_segment_run;
use crate::type_checker::profile::import_specs::{ModuleSpecifierClass, SourceModulePathPolicy};

/// PHP namespace path rules consumed by the generic import and re-export
/// rules. A `use` clause names a namespace, never a file, so nothing is
/// relative and no candidate file spellings exist; PSR-4 layout is the only
/// path evidence a namespace provides.
pub(crate) const PHP_SOURCE_MODULE_PATH_POLICY: SourceModulePathPolicy = SourceModulePathPolicy {
    classify_specifier,
    relative_candidate_paths,
    bare_module_matches_file,
    external_import_match_terms,
};

/// Every well-formed namespace path is a bare module; malformed text (empty
/// segments, empty path) stays unsupported so no rule infers syntax from it.
fn classify_specifier(specifier: &str) -> ModuleSpecifierClass {
    if super::profile::php_namespace_path_is_well_formed(specifier.trim_start_matches('\\')) {
        ModuleSpecifierClass::Bare
    } else {
        ModuleSpecifierClass::Unsupported
    }
}

fn relative_candidate_paths(_base: &str) -> Vec<String> {
    Vec::new()
}

/// PSR-4 places a namespace's declarations under a directory run spelled like
/// the namespace: `Illuminate\Support` lives at `.../Illuminate/Support/...`.
pub(crate) fn bare_module_matches_file(file_path: &str, source_module: &str) -> bool {
    let run = source_module.trim_start_matches('\\').replace('\\', "/");
    if run.is_empty() {
        return false;
    }
    path_contains_segment_run(&file_path.replace('\\', "/"), &run)
}

/// The namespace leaf, lower-cased, for external file-stem or directory
/// matching.
pub(crate) fn external_import_match_terms(module: &str) -> Vec<String> {
    module
        .trim_start_matches('\\')
        .rsplit('\\')
        .next()
        .filter(|leaf| !leaf.is_empty())
        .map(|leaf| vec![leaf.to_lowercase()])
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "module_paths_tests.rs"]
mod tests;
