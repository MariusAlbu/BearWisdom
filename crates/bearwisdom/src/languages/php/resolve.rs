// =============================================================================
// indexer/resolve/rules/php/mod.rs — PHP resolution rules
//
// Scope rules for PHP (7.4+, 8.x):
//
//   1. Chain-aware resolution: walk MemberChain following field/return types.
//   2. Scope chain walk: innermost scope → outermost, try {scope}.{target}
//   3. Same-namespace resolution: types in the same namespace are visible
//      without `use` (mirrors C# same-namespace visibility).
//   4. Use statement resolution: `use App\Models\User;` makes `User` visible.
//   5. Fully qualified names: backslash-separated names resolve directly
//      (normalized to dotted form in the index).
//
// PHP import model:
//   The PHP extractor emits EdgeKind::Imports refs for `use` declarations:
//     use App\Models\User;         → target_name = "User",  module = "App\Models\User"
//     use App\Models\User as U;    → target_name = "U",     module = "App\Models\User"
//
//   PHP namespaces use backslash as separator. The index normalizes these
//   to dotted form (e.g., "App\Models\User" → "App.Models.User") to be
//   consistent with the rest of the resolvers. We accept both forms in lookups.
//
// Adding new PHP features:
//   - Trait use → add to build_file_context (already EdgeKind::Imports in extractor).
//   - Enum backed types → extractor emits TypeRef; scope chain handles them.
// =============================================================================


// Re-export for test visibility (php_tests.rs uses `use super::*`).
pub(crate) use super::predicates::normalize_php_ns;

use super::{predicates, type_checker::PhpChecker};
use crate::type_checker::TypeChecker;
use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolInfo, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// PHP language resolver.
pub struct PhpResolver;

impl LanguageResolver for PhpResolver {
    fn language_ids(&self) -> &[&str] {
        &["php"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        let mut imports = Vec::new();

        // Extract the current namespace from the first Namespace symbol.
        let file_namespace = file.symbols.iter().find_map(|sym| {
            if sym.kind == crate::types::SymbolKind::Namespace {
                Some(sym.qualified_name.clone())
            } else {
                None
            }
        });

        // Extract `use` declarations from EdgeKind::Imports refs.
        // PHP extractor emits:
        //   use App\Models\User;       → target_name = "User",  module = "App\Models\User"
        //   use App\Models\User as U;  → target_name = "U",     module = "App\Models\User"
        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            let module = r.module.as_deref().unwrap_or(&r.target_name);

            // Normalize backslash separators to dots for index lookup consistency.
            let normalized_module = predicates::normalize_php_ns(module);

            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(normalized_module),
                alias: None,
                // PHP `use` is always an exact type import, not a wildcard.
                is_wildcard: false,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "php".to_string(),
            imports,
            file_namespace,
        }
    }

    fn resolve(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        let target = &ref_ctx.extracted_ref.target_name;
        let edge_kind = ref_ctx.extracted_ref.kind;

        // Skip import refs themselves — they're not symbol references.
        if edge_kind == EdgeKind::Imports {
            return None;
        }

        // Chain-aware resolution: if we have a structured MemberChain, walk it
        // step-by-step following field types.
        if let Some(chain_val) = &ref_ctx.extracted_ref.chain {
            if let Some(res) = PhpChecker.resolve_chain(
                chain_val, edge_kind, Some(file_ctx), ref_ctx, lookup,
            ) {
                return Some(res);
            }
        }

        // Normalize: strip `$this->` or `this.` prefix for member access.
        let effective_target = target
            .strip_prefix("$this->")
            .or_else(|| target.strip_prefix("this."))
            .unwrap_or(target);

        // Also normalize any backslash separators in the target itself.
        let normalized_target = predicates::normalize_php_ns(effective_target);
        let effective_target = normalized_target.as_str();

        // Step 1: Scope chain walk (innermost → outermost).
        // e.g., scope_chain = ["App.Controllers.UserController.store",
        //                       "App.Controllers.UserController",
        //                       "App.Controllers"]
        for scope in &ref_ctx.scope_chain {
            let candidate = format!("{scope}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if self.is_visible(file_ctx, ref_ctx, sym)
                    && predicates::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "php_scope_chain",
                        resolved_yield_type: None,
                    });
                }
            }
        }

        // Step 2: Same-namespace resolution.
        // In PHP, classes in the same namespace are visible without `use`.
        if let Some(ns) = &file_ctx.file_namespace {
            let candidate = format!("{ns}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if self.is_visible(file_ctx, ref_ctx, sym)
                    && predicates::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "php_same_namespace",
                        resolved_yield_type: None,
                    });
                }
            }
        }

        // Step 3: Use statement resolution.
        // `use App\Models\User;` → target "User" resolves to "App.Models.User"
        for import in &file_ctx.imports {
            if import.imported_name == effective_target {
                if let Some(module) = &import.module_path {
                    if let Some(sym) = lookup.by_qualified_name(module) {
                        if predicates::kind_compatible(edge_kind, &sym.kind) {
                            return Some(Resolution {
                                target_symbol_id: sym.id,
                                confidence: 1.0,
                                strategy: "php_use_statement",
                                resolved_yield_type: None,
                            });
                        }
                    }
                }
            }
        }

        // Step 4: Fully qualified name (target contains "\" or ".").
        if effective_target.contains('.') || effective_target.contains('\\') {
            if let Some(sym) = lookup.by_qualified_name(effective_target) {
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "php_qualified_name",
                        resolved_yield_type: None,
                    });
                }
            }
        }

        // Step 5: Global bare-name lookup for PHP global helper functions.
        //
        // PHP helper functions like `route()`, `trans()`, `auth()`, `view()`,
        // `config()` are declared at global scope — no namespace, so their
        // qualified name in the index is just their simple name (e.g. `route`).
        // These functions are called without `use` statements anywhere in the
        // project, so Steps 1-4 all miss them. We look them up directly via
        // `by_qualified_name(bare_name)` which finds external symbols indexed
        // from vendor/laravel/framework/src/Illuminate/Foundation/helpers.php
        // and similar global-helper files.
        //
        // Only triggers for Calls edges on simple (no-dot) names to avoid
        // matching class method names (those always carry a scope prefix).
        if edge_kind == EdgeKind::Calls && !effective_target.contains('.') {
            if let Some(sym) = lookup.by_qualified_name(effective_target) {
                if sym.kind == "function" {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 0.9,
                        strategy: "php_global_function",
                        resolved_yield_type: None,
                    });
                }
            }
        }

        // Step 6: Inheritance-chain walk for `$this->method()` calls.
        //
        // When a method call on `$this` could not be resolved by the scope
        // chain (Step 1), the method is likely defined on a parent class.
        // We walk the `inherits_map` upward from the calling class, trying
        // `{ancestor}.{method_name}` at each level (depth ≤ 10 to guard
        // against inheritance cycles in malformed source).
        //
        // Only fires for:
        //   - EdgeKind::Calls on a simple (no-dot) name
        //   - the call is on `$this` — detected via the chain's first segment
        //     being SelfRef, OR the original target had a `$this->` prefix
        //   - the scope chain has at least one class-level entry
        let is_this_call = {
            use crate::types::SegmentKind;
            // Check chain for SelfRef first segment (the normal PHP `$this->method()` pattern).
            let via_chain = ref_ctx
                .extracted_ref
                .chain
                .as_ref()
                .and_then(|c| c.segments.first())
                .map(|s| s.kind == SegmentKind::SelfRef)
                .unwrap_or(false);
            // Fallback: target still has the `$this->` prefix (emitted by some code paths).
            via_chain || target.starts_with("$this->") || target.starts_with("this.")
        };
        if edge_kind == EdgeKind::Calls
            && is_this_call
            && !effective_target.contains('.')
        {
            // The calling class is the first entry in the scope chain —
            // scope_chain is built from the source symbol's scope_path, which
            // is the enclosing class qname (e.g. "App.Services.SetupAccount").
            // scope_chain[0] is thus the class; scope_chain[1] is the namespace.
            let calling_class = ref_ctx
                .scope_chain
                .first()
                .map(|s| s.as_str());

            if let Some(mut class_qname) = calling_class {
                // Walk up at most 10 ancestors so cycles don't spin forever.
                for _ in 0..10 {
                    match lookup.parent_class_qname(class_qname) {
                        None => break,
                        Some(parent_qname) => {
                            let candidate = format!("{parent_qname}.{effective_target}");
                            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                                if self.is_visible(file_ctx, ref_ctx, sym)
                                    && predicates::kind_compatible(edge_kind, &sym.kind)
                                {
                                    return Some(Resolution {
                                        target_symbol_id: sym.id,
                                        confidence: 0.85,
                                        strategy: "php_inherited_method",
                                        resolved_yield_type: None,
                                    });
                                }
                            }
                            class_qname = parent_qname;
                        }
                    }
                }
            }
        }

        // Could not resolve deterministically — fall back to heuristic.
        None
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        // Import refs (`use` statements) — classify the `use` as external if the namespace is.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let import_path = ref_ctx
                .extracted_ref
                .module
                .as_deref()
                .unwrap_or(target);
            let normalized = predicates::normalize_php_ns(import_path);

            // Manifest-driven: check composer.json dependencies first.
            // Composer packages use `"vendor/package"` format (e.g., `"intervention/image"`).
            // PHP namespace roots are CamelCase (e.g., `"Intervention"`).
            if let Some(ctx) = project_ctx {
                if let Some(manifest) = ctx.manifests_for(ref_ctx.file_package_id).get(&ManifestKind::Composer) {
                    let ns_root = normalized.split('.').next().unwrap_or(&normalized);
                    if is_composer_package_match(ns_root, &manifest.dependencies) {
                        return Some(normalized);
                    }
                }
            }

            if predicates::is_external_php_namespace(&normalized, project_ctx) {
                return Some(normalized);
            }
            return None;
        }

        // PHP built-in functions — always external.
        if predicates::is_php_builtin(target) {
            return Some("php_core".to_string());
        }

        // Check use statement list for external namespaces.
        let mut best: Option<String> = None;

        for import in &file_ctx.imports {
            let ns = import.module_path.as_deref().unwrap_or("");
            if ns.is_empty() {
                continue;
            }

            // Manifest-driven check.
            let is_ext = if let Some(ctx) = project_ctx {
                if let Some(manifest) = ctx.manifests_for(ref_ctx.file_package_id).get(&ManifestKind::Composer) {
                    let ns_root = ns.split('.').next().unwrap_or(ns);
                    if is_composer_package_match(ns_root, &manifest.dependencies) {
                        true
                    } else {
                        predicates::is_external_php_namespace(ns, project_ctx)
                    }
                } else {
                    predicates::is_external_php_namespace(ns, project_ctx)
                }
            } else {
                predicates::is_external_php_namespace(ns, project_ctx)
            };

            if is_ext {
                if best.as_deref().is_none() || ns.len() > best.as_deref().unwrap().len() {
                    best = Some(ns.to_string());
                }
            }
        }

        best
    }

    fn is_visible(
        &self,
        file_ctx: &FileContext,
        _ref_ctx: &RefContext,
        target: &SymbolInfo,
    ) -> bool {
        let vis = target.visibility.as_deref().unwrap_or("public");
        match vis {
            "public" => true,
            "protected" => {
                // Accessible from same class or subclasses — approximate: allow.
                true
            }
            "private" => {
                // Only visible within the same file (same class).
                &*target.file_path == file_ctx.file_path
            }
            _ => true,
        }
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Check whether a PHP namespace root matches any composer.json package dependency.
///
/// Composer packages use `"vendor/package"` format (e.g., `"intervention/image"`).
/// PHP namespace roots are CamelCase (e.g., `"Intervention"`).
///
/// Matching strategy:
/// 1. Exact case-insensitive match of the namespace root against the package part
///    (after the `/`): `"Intervention"` matches `"intervention/image"` package part `"image"` — no.
///    Actually match against the last segment after `/`: vendor/package → package.
/// 2. Also try matching against the vendor segment before `/`.
///
/// For well-known mappings like `laravel/framework` → `Illuminate`, the
/// ALWAYS_EXTERNAL list in builtins handles them. This function catches packages
/// not in that list where the namespace root matches the composer package name.
fn is_composer_package_match(
    ns_root: &str,
    deps: &std::collections::HashSet<String>,
) -> bool {
    let ns_lower = ns_root.to_lowercase();
    for dep in deps {
        // `"vendor/package"` — check both vendor and package segments.
        let (vendor, package) = if let Some(slash) = dep.find('/') {
            (&dep[..slash], &dep[slash + 1..])
        } else {
            (dep.as_str(), dep.as_str())
        };
        // Normalize: replace hyphens with nothing for comparison (e.g., "my-package" → "mypackage").
        let vendor_lower = vendor.to_lowercase().replace('-', "");
        let package_lower = package.to_lowercase().replace('-', "");
        let ns_lower_nohyphen = ns_lower.replace('-', "");
        if vendor_lower == ns_lower_nohyphen || package_lower == ns_lower_nohyphen {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

