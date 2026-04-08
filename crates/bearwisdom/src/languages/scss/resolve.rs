// =============================================================================
// languages/scss/resolve.rs — SCSS resolution rules
//
// SCSS reference forms emitted by the extractor:
//
//   @include mixin-name(args)   → Calls,  target_name = "mixin-name",  module = None
//   @extend %placeholder        → Inherits, target_name = "%placeholder", module = None
//   @import 'path'              → Imports, target_name = last segment,  module = "path"
//   @use 'path' as alias        → Imports, target_name = last segment,  module = "path"
//   @forward 'path'             → Imports, target_name = last segment,  module = "path"
//   call_expression (fn call)   → Calls,  target_name = "function-name", module = None
//
// Resolution strategy:
//   1. Imports (@use / @import / @forward): record the module path in file
//      context. These are file-level declarations, not symbol references.
//   2. Mixin/function calls (@include, direct calls): look up the target name
//      via `lookup.by_name()`. SCSS symbols have bare names as qualified_name.
//   3. Same-file: mixin defined in the same file is always visible.
//   4. CSS built-in functions (color(), rgba(), etc.) are external.
// =============================================================================

use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

pub struct ScssResolver;

impl LanguageResolver for ScssResolver {
    fn language_ids(&self) -> &[&str] {
        &["scss"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        let mut imports = Vec::new();

        // Collect @use / @import / @forward paths from Imports refs.
        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            let module_path = r.module.clone().unwrap_or_else(|| r.target_name.clone());
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(module_path),
                alias: None,
                is_wildcard: false,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "scss".to_string(),
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

        // Skip import declarations — they declare scope, not symbol references.
        if edge_kind == EdgeKind::Imports {
            return None;
        }

        // Skip CSS built-in functions.
        if is_css_builtin(target) {
            return None;
        }

        // Step 1: Same-file — SCSS mixins/functions defined in the same file
        // are always in scope.
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name == *target {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "scss_same_file",
                });
            }
        }

        // Step 2: Search imported modules. For each @use/@import path, look for
        // the target in the corresponding file.
        for import in &file_ctx.imports {
            let Some(module_path) = &import.module_path else {
                continue;
            };
            for sym in lookup.in_file(module_path) {
                if sym.name == *target {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "scss_import",
                    });
                }
            }
        }

        // Step 3: Global lookup by name — covers partial imports and shared
        // design-system files accessible from anywhere in the project.
        let candidates = lookup.by_name(target);
        if let Some(sym) = candidates.into_iter().next() {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 0.85,
                strategy: "scss_global_name",
            });
        }

        None
    }

    fn infer_external_namespace(
        &self,
        _file_ctx: &FileContext,
        ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;
        // Mark Imports refs as their module path (moves them out of unresolved).
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let path = ref_ctx
                .extracted_ref
                .module
                .as_deref()
                .unwrap_or(target.as_str());
            return Some(path.to_string());
        }
        if is_css_builtin(target) {
            return Some("builtin".to_string());
        }
        None
    }
}

/// CSS / SCSS built-in function names that appear as call targets.
/// These are provided by the browser / Sass runtime — not project symbols.
fn is_css_builtin(name: &str) -> bool {
    matches!(
        name,
        // Sass color functions
        "rgb" | "rgba" | "hsl" | "hsla" | "color" | "lighten" | "darken"
            | "saturate" | "desaturate" | "mix" | "opacify" | "transparentize"
            | "fade-in" | "fade-out" | "invert" | "complement" | "adjust-color"
            | "scale-color" | "change-color" | "ie-hex-str" | "adjust-hue"
            | "grayscale" | "alpha" | "opacity" | "red" | "green" | "blue"
            | "hue" | "saturation" | "lightness" | "whiteness" | "blackness"
            // Sass math/string/list/map/selector/meta functions
            | "abs" | "ceil" | "floor" | "round" | "max" | "min" | "random"
            | "percentage" | "unit" | "unitless" | "comparable" | "sqrt"
            | "pow" | "log" | "cos" | "sin" | "tan" | "acos" | "asin" | "atan"
            | "atan2" | "hypot" | "clamp"
            | "quote" | "unquote" | "str-length" | "str-insert" | "str-index"
            | "str-slice" | "to-upper-case" | "to-lower-case" | "unique-id"
            | "length" | "nth" | "set-nth" | "join" | "append" | "zip" | "index"
            | "is-bracketed" | "list-separator"
            | "map-get" | "map-merge" | "map-remove" | "map-keys" | "map-values"
            | "map-has-key" | "keywords"
            | "type-of" | "inspect" | "variable-exists" | "global-variable-exists"
            | "function-exists" | "mixin-exists" | "content-exists"
            | "get-function" | "call" | "if"
            // CSS native functions
            | "var" | "calc" | "env" | "url" | "attr" | "counter" | "counters"
            | "format" | "local" | "linear-gradient" | "radial-gradient"
            | "repeating-linear-gradient" | "repeating-radial-gradient"
            | "conic-gradient" | "image-set" | "cross-fade"
            | "translate" | "translateX" | "translateY" | "translateZ"
            | "translate3d" | "scale" | "scaleX" | "scaleY" | "rotate"
            | "rotateX" | "rotateY" | "rotateZ" | "skew" | "skewX" | "skewY"
            | "perspective" | "matrix" | "matrix3d"
            | "blur" | "brightness" | "contrast" | "drop-shadow"
            | "grayscale-filter" | "hue-rotate" | "invert-filter" | "opacity-filter"
            | "saturate-filter" | "sepia"
    )
}
