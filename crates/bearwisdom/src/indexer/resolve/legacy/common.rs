// =============================================================================
// indexer/resolve/engine/common.rs — shared external-classification helpers
//
// `infer_external_common` decides whether an unresolved ref should be
// classified as belonging to an external namespace (third-party package,
// runtime ambient, declared dependency). Language plugins call it from
// `LanguageEngineHooks::classify_external`.
//
// The companion symbol-resolution helper `resolve_common` lived here too
// until every language hook adopted `DefaultResolver`. That function is
// gone; the symbol-resolution path is centralized in
// `type_checker/core/default_resolver.rs`.
// =============================================================================

use crate::indexer::project_context::ProjectContext;
use crate::types::EdgeKind;

use super::{FileContext, RefContext};

/// Common external namespace inference for tier-2 languages.
///
/// Classifies refs as external when:
///   1. The ref is an import (the import path IS the namespace)
///   2. The target name is a known builtin/external
///   3. The target was imported from a non-relative module (bare-name walk)
///   4. A module-qualified ref matches an import path
///   5. Chain-root propagation: if the root of a MemberChain was imported
///      from a non-relative module, the entire chain is external
pub fn infer_external_common(
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    project_ctx: Option<&ProjectContext>,
    is_builtin: fn(&str) -> bool,
) -> Option<String> {
    let target = &ref_ctx.extracted_ref.target_name;

    // Import refs: the import path is the external namespace.
    if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
        let ns = ref_ctx.extracted_ref.module.as_deref().unwrap_or(target);
        return Some(ns.to_string());
    }

    // Known builtin/stdlib function or type.
    if is_builtin(target) {
        return Some("builtin".to_string());
    }

    // Module-qualified ref where the module matches an import path:
    // the ref comes from an external dependency.
    if let Some(module) = &ref_ctx.extracted_ref.module {
        for import in &file_ctx.imports {
            let Some(module_path) = &import.module_path else {
                continue;
            };
            if import.imported_name == *module || module_path.contains(module.as_str()) {
                return Some(module_path.clone());
            }
        }
    }

    // Bare-name import walk: if the target was imported from a non-relative
    // module, it's external. This handles `import Test.Hspec` → `it` is
    // external, `from django.db import models` → `models` is external, etc.
    //
    // For wildcard imports (`import Module` with is_wildcard=true), any
    // unresolved name *could* come from that module. We classify it as
    // external since resolve() already tried and failed to find it locally.
    let simple = target.split('.').next().unwrap_or(target);
    for import in &file_ctx.imports {
        let Some(module_path) = &import.module_path else {
            continue;
        };
        // Skip relative imports — those are project-internal.
        if module_path.starts_with('.') || module_path.starts_with("crate") {
            continue;
        }

        // Determine if we have a meaningful manifest — an empty manifests map
        // means no ecosystem parser exists for this language (Haskell, OCaml, etc.).
        // In that case, treat non-relative imports as external since we have no
        // way to distinguish project-local from third-party.
        //
        // For per-package isolation, `manifests_for(package_id)` returns only
        // the source file's own package's manifests — so `server/` files don't
        // see deps that only `e2e/` declares.
        let pkg_id = ref_ctx.file_package_id;
        let pkg_manifests = project_ctx.map(|ctx| ctx.manifests_for(pkg_id));
        let has_manifest = pkg_manifests.map(|m| !m.is_empty()).unwrap_or(false);

        // Named import match: `from foo import Bar` → target "Bar" matches.
        if !import.is_wildcard && import.imported_name == simple {
            if has_manifest {
                if is_manifest_dependency(project_ctx.unwrap(), pkg_id, module_path) {
                    return Some(module_path.clone());
                }
                // Manifest exists but dep not in it — project-internal.
                continue;
            }
            // No manifest at all — conservatively treat non-relative as external.
            return Some(module_path.clone());
        }

        // Alias match: `import qualified Data.Map as Map` → target "Map" matches alias.
        if let Some(alias) = &import.alias {
            if alias == simple {
                if has_manifest {
                    if is_manifest_dependency(project_ctx.unwrap(), pkg_id, module_path) {
                        return Some(module_path.clone());
                    }
                    continue;
                }
                return Some(module_path.clone());
            }
        }

        // Wildcard import: `import Module` (no selective list) — any unresolved
        // bare name could come from this module. Fire when:
        //   (a) manifest confirms the module is a dependency, OR
        //   (b) no manifest parser exists for this language at all
        if import.is_wildcard {
            if has_manifest {
                if is_manifest_dependency(project_ctx.unwrap(), pkg_id, module_path) {
                    return Some(module_path.clone());
                }
            } else {
                // No manifest parser → can't distinguish, but resolve() already
                // failed to find it locally, so classify as external.
                return Some(module_path.clone());
            }
        }
    }

    // Chain-root propagation: if the ref has a MemberChain and the root
    // segment was imported from a non-relative module, classify the whole
    // chain as external.
    if let Some(chain_ref) = &ref_ctx.extracted_ref.chain {
        if chain_ref.segments.len() >= 2 {
            let root = &chain_ref.segments[0].name;
            for import in &file_ctx.imports {
                if import.imported_name != root.as_str() {
                    continue;
                }
                let Some(module_path) = &import.module_path else {
                    continue;
                };
                if module_path.starts_with('.') || module_path.starts_with("crate") {
                    continue;
                }
                let is_ext = match project_ctx {
                    Some(ctx) => is_manifest_dependency(ctx, ref_ctx.file_package_id, module_path),
                    None => true,
                };
                if is_ext {
                    return Some(format!("{}.*", module_path));
                }
            }
        }
    }

    None
}

/// Check if a module path corresponds to a known dependency in the manifest
/// visible to `package_id`.
///
/// Extracts the root package name from the module path and checks the
/// per-package manifests when `package_id` is known — falls back to the
/// whole-project union for files without a package (root configs, shared
/// scripts) or for legacy single-unit contexts.
///
/// Handles common naming conventions:
///   - Python: `django.db` → root "django", also checks "django" with hyphen swap
///   - Rust: `tokio::runtime` → root "tokio"
///   - Haskell: `Test.Hspec` → root "hspec" (lowercase)
///   - Elixir: `Phoenix.Controller` → root "phoenix" (lowercase)
fn is_manifest_dependency(
    ctx: &ProjectContext,
    package_id: Option<i64>,
    module_path: &str,
) -> bool {
    // Extract the root segment — the part before the first separator.
    let root = module_path
        .split('.')
        .next()
        .and_then(|s| s.split("::").next())
        .unwrap_or(module_path);

    let root_lower = root.to_lowercase();
    let manifests = ctx.manifests_for(package_id);
    for manifest in manifests.values() {
        if manifest.dependencies.contains(root)
            || manifest.dependencies.contains(&root_lower)
            || manifest
                .dependencies
                .contains(&root_lower.replace('_', "-"))
            || manifest
                .dependencies
                .contains(&root_lower.replace('-', "_"))
        {
            return true;
        }
        // Also check if any dep starts with the root as a prefix
        // (handles scoped packages like `ecto_sql` matching root `Ecto`).
        for dep in &manifest.dependencies {
            if dep.split('_').next() == Some(&root_lower) {
                return true;
            }
        }
    }
    false
}
