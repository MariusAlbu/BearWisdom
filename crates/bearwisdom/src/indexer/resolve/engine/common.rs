// =============================================================================
// indexer/resolve/engine/common.rs — shared tier-2 resolver helpers
//
// Free functions language resolvers use to handle the common "scope walk +
// import lookup + qualified-name fallback" pattern. Each language plugin can
// call resolve_common / infer_external_common instead of re-implementing the
// same lookup cascade. The lang_prefix argument provides per-language
// strategy attribution for diagnostics.
// =============================================================================

use crate::indexer::project_context::ProjectContext;
use crate::types::EdgeKind;

use super::{FileContext, RefContext, Resolution, SymbolLookup};

// ---------------------------------------------------------------------------
// Shared resolution helpers for tier-2 language resolvers
// ---------------------------------------------------------------------------

/// Common resolution logic for languages that follow the standard pattern.
///
/// Steps (highest confidence first):
///   1. Module-qualified lookup: ref has `module` field → try `{module}.{target}`
///   2. Import-based: find target in imported modules via file context
///   3. Scope chain walk: try `{scope}.{target}` for each scope
///   4. Same-file: find target among symbols in the current file
///   5. Qualified name: target contains `.` → direct qname lookup
///
/// Does NOT include a by-name fallback — that's the heuristic resolver's job.
/// This prevents low-confidence false matches from intercepting the heuristic's
/// module-aware matching.
pub fn resolve_common(
    lang_prefix: &'static str,
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    lookup: &dyn SymbolLookup,
    kind_compatible: fn(EdgeKind, &str) -> bool,
) -> Option<Resolution> {
    let target = &ref_ctx.extracted_ref.target_name;
    let edge_kind = ref_ctx.extracted_ref.kind;

    // Skip import refs — they declare scope, not symbol references.
    if edge_kind == EdgeKind::Imports {
        return None;
    }

    // Step 1: Module-qualified lookup.
    // If ref has module="List" and target="map", try "List.map" as qname.
    if let Some(module) = &ref_ctx.extracted_ref.module {
        // Try direct qualified name: module.target
        let candidates = [
            format!("{module}.{target}"),
            format!("{module}::{target}"),
            format!("{module}/{target}"),
            format!("{module}:{target}"),
        ];
        for candidate in &candidates {
            if let Some(sym) = lookup.by_qualified_name(candidate) {
                if kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: concat_strategy(lang_prefix, "module_qualified"),
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }

        // Try finding target in files that match the module name
        let by_name = lookup.by_name(target);
        for sym in by_name {
            let file_lower = sym.file_path.to_lowercase();
            let module_lower = module.to_lowercase();
            // File stem or path segment matches module name
            if file_stem_matches(&file_lower, &module_lower)
                && kind_compatible(edge_kind, &sym.kind)
            {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.95,
                    strategy: concat_strategy(lang_prefix, "module_file"),
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }
    }

    // Step 2: Import-based resolution.
    // Check if target matches an imported name, then find it in the imported module.
    for import in &file_ctx.imports {
        let Some(module_path) = &import.module_path else {
            continue;
        };

        // Wildcard import: all names from module are in scope
        if import.is_wildcard {
            let by_name = lookup.by_name(target);
            for sym in by_name {
                let file_lower = sym.file_path.to_lowercase();
                let mod_lower = module_path.to_lowercase();
                let last_seg = mod_lower.rsplit('/').next()
                    .unwrap_or(&mod_lower)
                    .rsplit('.')
                    .next()
                    .unwrap_or(&mod_lower);
                if file_stem_matches(&file_lower, last_seg)
                    && kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 0.95,
                        strategy: concat_strategy(lang_prefix, "import"),
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
            continue;
        }

        // Named import: target matches the imported name or its local alias.
        // When an alias is present (e.g. `import { Foo as Bar }`, or Fortran
        // `use m, only: local => source`), the target is the *local* name but
        // the symbol in the index uses the *source* name — look up by
        // `imported_name` when the alias matched.
        let matches_direct = import.imported_name == *target;
        let matches_alias = import.alias.as_deref() == Some(target);
        if matches_direct || matches_alias {
            // When the local alias matched, the actual symbol name is
            // `imported_name`; otherwise use the target as-is.
            let lookup_name: &str = if matches_alias {
                &import.imported_name
            } else {
                target
            };
            let by_name = lookup.by_name(lookup_name);
            for sym in by_name {
                if kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 0.95,
                        strategy: concat_strategy(lang_prefix, "import"),
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }
    }

    // Step 3: Scope chain walk.
    for scope in &ref_ctx.scope_chain {
        let candidate = format!("{scope}.{target}");
        if let Some(sym) = lookup.by_qualified_name(&candidate) {
            if kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: concat_strategy(lang_prefix, "scope_chain"),
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }
    }

    // Step 4: Same-file resolution.
    for sym in lookup.in_file(&file_ctx.file_path) {
        if sym.name == *target && kind_compatible(edge_kind, &sym.kind) {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 1.0,
                strategy: concat_strategy(lang_prefix, "same_file"),
                resolved_yield_type: None,
                flow_emit: None,
            });
        }
    }

    // Step 5: Fully qualified name (target contains dots).
    if target.contains('.') || target.contains("::") || target.contains('/') {
        if let Some(sym) = lookup.by_qualified_name(target) {
            if kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: concat_strategy(lang_prefix, "qualified_name"),
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }
    }

    // No deterministic resolution — let heuristic handle it.
    None
}

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
        let ns = ref_ctx
            .extracted_ref
            .module
            .as_deref()
            .unwrap_or(target);
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
            if import.imported_name == *module
                || module_path.contains(module.as_str())
            {
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
        // For per-package isolation (M2), `manifests_for(package_id)` returns
        // only the source file's own package's manifests — so `server/` files
        // don't see deps that only `e2e/` declares.
        let pkg_id = ref_ctx.file_package_id;
        let pkg_manifests = project_ctx.map(|ctx| ctx.manifests_for(pkg_id));
        let has_manifest = pkg_manifests
            .map(|m| !m.is_empty())
            .unwrap_or(false);

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
                    Some(ctx) => {
                        is_manifest_dependency(ctx, ref_ctx.file_package_id, module_path)
                    }
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
            || manifest.dependencies.contains(&root_lower.replace('_', "-"))
            || manifest.dependencies.contains(&root_lower.replace('-', "_"))
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

/// Check if a file path's stem matches a module name.
fn file_stem_matches(file_path_lower: &str, module_lower: &str) -> bool {
    let normalized = file_path_lower.replace('\\', "/");
    // Check file stem: "src/lists.erl" stem is "lists"
    if let Some(basename) = normalized.rsplit('/').next() {
        if let Some(stem) = basename.rsplit_once('.').map(|(s, _)| s) {
            if stem == module_lower {
                return true;
            }
        }
    }
    // Check path segment: "src/lists/mod.rs".
    // External paths carry an "ext:<lang>:<pkg>" prefix segment — strip the
    // colon-delimited prefix so "ext:ocaml:ctypes" matches module "ctypes".
    normalized.split('/').any(|seg| {
        seg == module_lower
            || seg.split(':').last().map_or(false, |tail| tail == module_lower)
    })
}

/// Strategy name helper — returns a leaked &'static str for diagnostics.
/// Uses a fixed set of known suffixes to avoid allocation.
fn concat_strategy(prefix: &'static str, suffix: &str) -> &'static str {
    // For diagnostics only — use the prefix as a fallback.
    // The full "{prefix}_{suffix}" string can't be &'static without leaking,
    // so we return just the suffix which is always a literal.
    match suffix {
        "module_qualified" => match prefix {
            "erlang" => "erlang_module_qualified",
            "ocaml" => "ocaml_module_qualified",
            "haskell" => "haskell_module_qualified",
            "r" => "r_module_qualified",
            "clojure" => "clojure_module_qualified",
            "pascal" => "pascal_module_qualified",
            "fortran" => "fortran_module_qualified",
            "matlab" => "matlab_module_qualified",
            "powershell" => "powershell_module_qualified",
            "fsharp" => "fsharp_module_qualified",
            _ => "common_module_qualified",
        },
        "module_file" => "common_module_file",
        "import" => "common_import",
        "scope_chain" => "common_scope_chain",
        "same_file" => "common_same_file",
        "qualified_name" => "common_qualified_name",
        _ => "common_resolved",
    }
}
