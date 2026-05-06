// =============================================================================
// gdscript/resolve.rs — GDScript resolution rules
//
// GDScript module system:
//   - `extends BaseClass` — inherit from a built-in or script class.
//   - `class_name ClassName` — register a script as a global class name.
//   - `preload("res://path/to/Script.gd")` — load at parse time, returns a
//     GDScript reference stored in a constant.
//   - `load("res://path/to/Script.gd")` — load at runtime.
//   - Globally registered class names (via `class_name`) are visible everywhere
//     without an explicit preload/load.
//
// Resolution strategy:
//   1. Scope chain walk (inner class → outer class → script).
//   2. Same-file resolution (constants, variables, functions in the same script).
//   3. Project-wide name lookup (global class_name registrations).
// =============================================================================

use super::predicates;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution,
    SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// GDScript language resolver.
pub struct GDScriptResolver;

impl LanguageResolver for GDScriptResolver {
    fn language_ids(&self) -> &[&str] {
        &["gdscript"]
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
            // target_name holds the preload/load path or the base class name.
            let module_path = r.module.clone().or_else(|| Some(r.target_name.clone()));
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path,
                alias: None,
                // preload / load bring in the whole script — treat as wildcard.
                is_wildcard: true,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "gdscript".to_string(),
            imports,
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

        // Language-keyword tokens that tree-sitter sometimes emits as ref
        // targets: `super` is the parent-class access keyword; `$` is the
        // node-path sugar for `get_node()`. Neither maps to a user symbol.
        if matches!(target.as_str(), "super" | "$") {
            return None;
        }

        // Bare-name lookup with synthetic-symbol preference. Built-in
        // GDScript globals (`print`, `Vector2`, `Node`, ...) live under
        // `ext:godot-api:` paths emitted by the godot_api walker. Run
        // before resolve_common so a bare token binds to the real walker
        // symbol when it exists.
        if !target.contains('.') && !target.contains('/') {
            let mut synthetic_match = None;
            let mut internal_match = None;
            for sym in lookup.by_name(target) {
                if !predicates::kind_compatible(edge_kind, &sym.kind) {
                    continue;
                }
                if sym.file_path.starts_with("ext:") {
                    synthetic_match = Some(sym);
                    break;
                } else if internal_match.is_none() {
                    internal_match = Some(sym);
                }
            }
            if let Some(sym) = synthetic_match.or(internal_match) {
                let strategy = if sym.file_path.starts_with("ext:") {
                    "gdscript_synthetic_global"
                } else {
                    "gdscript_internal_global"
                };
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: if strategy == "gdscript_synthetic_global" { 0.95 } else { 0.9 },
                    strategy,
                    resolved_yield_type: None,
                });
            }
        }

        engine::resolve_common("gdscript", file_ctx, ref_ctx, lookup, predicates::kind_compatible)
    }

    fn infer_external_namespace(
        &self,
        _file_ctx: &FileContext,
        _ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        // No predicate-driven classification — godot_api walker emits real
        // symbols and resolve() above binds them. Bare names that reach this
        // point exhausted same-file, scope, and walker lookups; leave
        // unresolved rather than blanket-classifying as `builtin`.
        None
    }
}
