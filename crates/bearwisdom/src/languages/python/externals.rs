// =============================================================================
// languages/python/externals.rs — external-namespace classification
//
// Classifies whether a module path / referenced symbol resolves outside the
// project boundary. Consulted by `PythonResolver::infer_external_namespace*`.
//
// Hierarchy of checks, cheapest-first:
//   1. Manifest declared dependency (pyproject.toml).
//   2. `is_manifest_python_package` (handles underscore/hyphen rewrites).
//   3. `SymbolLookup::has_in_namespace` — structural fallback that catches
//      transitive deps and stdlib-version gaps without growing a hardcoded set.
//   4. No manifest visible — permissive: treat as external.
// =============================================================================

use super::predicates;
use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolLookup};
use crate::types::EdgeKind;

/// Check whether a Python package root is an external dependency using the project manifest.
fn is_manifest_python_package(ctx: &ProjectContext, name: &str) -> bool {
    ctx.has_dependency(ManifestKind::PyProject, name)
        || ctx.has_dependency(ManifestKind::PyProject, &name.replace('_', "-"))
}

/// Returns Some(namespace) when `module` should be treated as external. Walks
/// the manifest, the stdlib list, and finally a `has_in_namespace` structural
/// check that catches transitive deps and stdlib-version gaps without growing
/// the hardcoded set (e.g. `httpx` pulled in via `httpx-oauth`, or `zoneinfo`
/// added in 3.9).
fn module_is_external(
    project_ctx: Option<&ProjectContext>,
    pkg_id: Option<i64>,
    lookup: Option<&dyn SymbolLookup>,
    module: &str,
) -> Option<String> {
    let root = module.split('.').next().unwrap_or(module);
    if let Some(ctx) = project_ctx {
        if let Some(manifest) = ctx.manifests_for(pkg_id).get(&ManifestKind::PyProject) {
            if manifest.dependencies.contains(root)
                || manifest.dependencies.contains(&root.replace('_', "-"))
            {
                return Some(module.to_string());
            }
        }
        if is_manifest_python_package(ctx, root) {
            return Some(module.to_string());
        }
    }
    if let Some(lookup) = lookup {
        // No internal symbols under this module name → external (transitive
        // dep, package-rename, or runtime-only library).
        if !lookup.has_in_namespace(root) {
            return Some(module.to_string());
        }
    }
    if project_ctx.is_none() {
        // No manifest visible — be permissive (matches the prior behaviour).
        return Some(module.to_string());
    }
    None
}

pub(super) fn infer_external_inner(
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    project_ctx: Option<&ProjectContext>,
    lookup: Option<&dyn SymbolLookup>,
) -> Option<String> {
    let target = &ref_ctx.extracted_ref.target_name;
    let pkg_id = ref_ctx.file_package_id;

    if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
        let module = ref_ctx.extracted_ref.module.as_deref().unwrap_or(target);
        if predicates::is_relative_import(module) {
            return None;
        }
        return module_is_external(project_ctx, pkg_id, lookup, module);
    }

    let simple = target.split('.').next().unwrap_or(target);
    for import in &file_ctx.imports {
        if import.imported_name != simple {
            continue;
        }
        let Some(ref mod_path) = import.module_path else {
            continue;
        };
        if predicates::is_relative_import(mod_path) {
            continue;
        }
        if let Some(ns) = module_is_external(project_ctx, pkg_id, lookup, mod_path) {
            return Some(ns);
        }
    }
    None
}
