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


use super::builtins;
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
        if builtins::is_swift_builtin(target) {
            return None;
        }

        let effective_target = target
            .strip_prefix("self.")
            .unwrap_or(target);

        // Step 1: Scope chain walk.
        for scope in &ref_ctx.scope_chain {
            let candidate = format!("{scope}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if builtins::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "swift_scope_chain",
                    });
                }
            }
        }

        // Step 2: Same-file resolution.
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name == effective_target && builtins::kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "swift_same_file",
                });
            }
        }

        // Step 3: Fully qualified name.
        if effective_target.contains('.') {
            if let Some(sym) = lookup.by_qualified_name(effective_target) {
                if builtins::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "swift_qualified_name",
                    });
                }
            }
        }

        // Step 4: Simple name lookup.
        for sym in lookup.by_name(effective_target) {
            if builtins::kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.85,
                    strategy: "swift_by_name",
                });
            }
        }

        None
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        // Import statements.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let module = ref_ctx.extracted_ref.module.as_deref().unwrap_or(target);
            let root = module.split('.').next().unwrap_or(module);
            if builtins::is_external_swift_module(root) {
                return Some(root.to_string());
            }
            return None;
        }

        // Swift stdlib builtins.
        if builtins::is_swift_builtin(target) {
            return Some("Swift".to_string());
        }

        // Walk imports: if the target was imported from a known-external module.
        for import in &file_ctx.imports {
            let module = import.module_path.as_deref().unwrap_or("");
            if module.is_empty() {
                continue;
            }
            let root = module.split('.').next().unwrap_or(module);
            if builtins::is_external_swift_module(root) {
                // The unresolved name could come from this imported module.
                return Some(root.to_string());
            }
        }

        // Module-qualified target: `UIKit.UIView` → root is UIKit.
        if target.contains('.') {
            let root = target.split('.').next().unwrap_or(target);
            if builtins::is_external_swift_module(root) {
                return Some(root.to_string());
            }
        }

        None
    }
}
