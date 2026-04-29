// =============================================================================
// indexer/resolve/rules/swift/mod.rs — Swift resolution rules
//
// Scope rules for Swift:
//
//   1. Scope chain walk: innermost type/function → outermost.
//   2. Same-file resolution: Swift files share a module — all top-level types
//      are visible within the same module without import.
//   3. Import resolution: `import Foundation` / `import UIKit` → external.
//   4. Fully qualified names: dot-separated names resolve directly.
//
// Swift import model:
//   `import Foundation`          → whole-module import
//   `import UIKit.UIView`        → submodule import
//   `import class Foundation.URL` → declaration import
//
// The extractor emits EdgeKind::Imports with:
//   target_name = module name (e.g., "Foundation", "UIKit")
//   module      = submodule path if present (e.g., "UIKit.UIView")
// =============================================================================


use super::predicates;
use crate::ecosystem::manifest::ManifestKind;
use crate::type_checker::chain::{
    self, ChainConfig, NamespaceLookup, identity_normalize,
};
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// Swift language resolver.
pub struct SwiftResolver;

impl LanguageResolver for SwiftResolver {
    fn language_ids(&self) -> &[&str] {
        &["swift"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        let mut imports = Vec::new();

        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            let module = r.module.as_deref().unwrap_or(&r.target_name);
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(module.to_string()),
                alias: None,
                is_wildcard: false,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "swift".to_string(),
            imports,
            // Swift has no file-level namespace; module is the product name.
            file_namespace: None,
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

        if edge_kind == EdgeKind::Imports {
            return None;
        }

        // Swift builtins are never in the project index.
        if predicates::is_swift_builtin(target) {
            return None;
        }

        // Chain-aware resolution: walk MemberChain following field/return types.
        // Swift has generics but no wildcard-import namespace lookup.
        if let Some(chain_val) = &ref_ctx.extracted_ref.chain {
            let config = ChainConfig {
                strategy_prefix: "swift",
                normalize_type: identity_normalize,
                has_self_ref: true,
                enclosing_type_kinds: &["class", "struct", "enum", "protocol"],
                static_type_kinds: &["class", "struct", "enum", "protocol", "type_alias"],
                use_generics: true,
                namespace_lookup: NamespaceLookup::None,
                kind_compatible: predicates::kind_compatible,
            };
            if let Some(res) = chain::resolve_via_chain(
                &config, chain_val, edge_kind, Some(file_ctx), ref_ctx, lookup,
            ) {
                return Some(res);
            }
        }

        let effective_target = target
            .strip_prefix("self.")
            .unwrap_or(target);

        // Step 1: Scope chain walk.
        for scope in &ref_ctx.scope_chain {
            let candidate = format!("{scope}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "swift_scope_chain",
                        resolved_yield_type: None,
                    });
                }
            }
        }

        // Step 2: Same-file resolution.
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name == effective_target && predicates::kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "swift_same_file",
                    resolved_yield_type: None,
                });
            }
        }

        // Step 3: Fully qualified name.
        if effective_target.contains('.') {
            if let Some(sym) = lookup.by_qualified_name(effective_target) {
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "swift_qualified_name",
                        resolved_yield_type: None,
                    });
                }
            }
        }

        // Step 4: Simple name lookup.
        for sym in lookup.by_name(effective_target) {
            if predicates::kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.85,
                    strategy: "swift_by_name",
                    resolved_yield_type: None,
                });
            }
        }

        // Swift bare-name fallback. Final language in the cross-language
        // template (PRs 31, 35-40, plus Lua, Go, Rust, Kotlin, Ruby,
        // Dart). Swift's protocol extensions and global functions are
        // callable by bare name across modules. Gated by `.swift`
        // file extension.
        if matches!(edge_kind, EdgeKind::Calls | EdgeKind::TypeRef | EdgeKind::Instantiates)
            && ref_ctx.extracted_ref.module.is_none()
            && !target.contains('.')
        {
            for sym in lookup.by_name(target) {
                if !predicates::kind_compatible(edge_kind, &sym.kind) {
                    continue;
                }
                let path = &sym.file_path;
                let is_swift = path.ends_with(".swift");
                if !is_swift {
                    continue;
                }
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.80,
                    strategy: "swift_bare_name",
                    resolved_yield_type: None,
                });
            }
        }

        None
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        infer_external_inner(file_ctx, ref_ctx, project_ctx, None)
    }

    fn infer_external_namespace_with_lookup(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
        lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        infer_external_inner(file_ctx, ref_ctx, project_ctx, Some(lookup))
    }
}

fn manifest_dep_match(
    project_ctx: Option<&ProjectContext>,
    pkg_id: Option<i64>,
    root: &str,
) -> bool {
    let Some(ctx) = project_ctx else { return false };
    let Some(manifest) = ctx.manifests_for(pkg_id).get(&ManifestKind::SwiftPM) else {
        return false;
    };
    let root_lower = root.to_lowercase();
    manifest.dependencies.iter().any(|d| {
        let d_lower = d.to_lowercase();
        d_lower == root_lower || d_lower.trim_start_matches("swift-") == root_lower.as_str()
    })
}

fn module_is_external(
    project_ctx: Option<&ProjectContext>,
    pkg_id: Option<i64>,
    lookup: Option<&dyn SymbolLookup>,
    root: &str,
) -> bool {
    if manifest_dep_match(project_ctx, pkg_id, root) {
        return true;
    }
    if predicates::is_external_swift_module(root) {
        return true;
    }
    // Structural: any imported module the index has no symbols for is
    // external (Apple first-party frameworks, third-party SDKs not
    // pulled into the project's symbol set, etc.).
    if let Some(lookup) = lookup {
        if !lookup.has_in_namespace(root) {
            return true;
        }
    }
    false
}

fn infer_external_inner(
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    project_ctx: Option<&ProjectContext>,
    lookup: Option<&dyn SymbolLookup>,
) -> Option<String> {
    let target = &ref_ctx.extracted_ref.target_name;
    let pkg_id = ref_ctx.file_package_id;

    if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
        let module = ref_ctx.extracted_ref.module.as_deref().unwrap_or(target);
        let root = module.split('.').next().unwrap_or(module);
        if module_is_external(project_ctx, pkg_id, lookup, root) {
            return Some(root.to_string());
        }
        return None;
    }

    if predicates::is_swift_builtin(target) {
        return Some("Swift".to_string());
    }

    for import in &file_ctx.imports {
        let Some(module) = import.module_path.as_deref() else {
            continue;
        };
        if module.is_empty() {
            continue;
        }
        let root = module.split('.').next().unwrap_or(module);
        if module_is_external(project_ctx, pkg_id, lookup, root) {
            return Some(root.to_string());
        }
    }

    if target.contains('.') {
        let root = target.split('.').next().unwrap_or(target);
        if module_is_external(project_ctx, pkg_id, lookup, root) {
            return Some(root.to_string());
        }
    }

    None
}
