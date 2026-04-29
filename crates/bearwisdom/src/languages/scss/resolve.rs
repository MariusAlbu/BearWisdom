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
//   5. Sass built-in modules (@use 'sass:math') are external.
//      `module.$variable` calls where the module is a sass:* import resolve as
//      external without entering the index.
// =============================================================================

use super::predicates;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution,
    SymbolLookup,
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

        // Property-value `call_expression` refs (tagged by the extractor):
        // these are CSS/SCSS built-in function evaluations
        // (`rgb(…)`, `calc(…)`, `color-mix(…)`, `steps(…)`, …) and should
        // not hit the project symbol index. The previous approach — a
        // hardcoded `is_scss_builtin` list — drifted (missed `color-mix`,
        // `oklch`, and every future CSS Level 5+ addition). The hint
        // works generically: the resolver trusts the extractor's
        // positional distinction between `@include mixin(…)` and
        // property-value `fn(…)`.
        if let Some(module) = &ref_ctx.extracted_ref.module {
            if module == super::extract::SCSS_CSS_FN_HINT {
                return None;
            }
        }

        // Legacy safety net: kept until extractor migration fully settled.
        if predicates::is_scss_builtin(target) {
            return None;
        }

        // Skip references into Sass built-in modules (sass:math, sass:color …).
        // These are module-qualified accesses like `math.$pi` or `math.div(…)`.
        if let Some(module) = &ref_ctx.extracted_ref.module {
            if predicates::is_sass_builtin_module(module) {
                return None;
            }
        }

        if let Some(res) = engine::resolve_common(
            "scss", file_ctx, ref_ctx, lookup, predicates::kind_compatible,
        ) {
            return Some(res);
        }

        // SCSS bare-name fallback. SCSS's legacy `@import` model has no
        // per-file namespacing — `@function` / `@mixin` definitions are
        // global within the compilation unit, and user code calls them
        // by bare name (`@include assert-equal(...)`, `color-contrast(...)`)
        // without a `module` qualifier. `engine::resolve_common`'s
        // module/import/scope/same-file path can't bind these unless
        // the import map names the source. Index-wide name lookup is
        // the right resolver step for this language — it's the
        // structural counterpart of TypeScript's @types-driven global
        // access, not a cross-language bypass.
        //
        // Skip refs that already carry a module hint (handled above)
        // and refs whose target is a CSS/Sass built-in (also handled).
        // The kind compatibility check screens out variable-style refs
        // matching unrelated identifiers in other languages.
        if ref_ctx.extracted_ref.module.is_some() {
            return None;
        }
        for sym in lookup.by_name(target) {
            if !predicates::kind_compatible(edge_kind, &sym.kind) {
                continue;
            }
            // Only resolve to SCSS-defined symbols. Cross-language
            // collisions (a Python `assert_equal` shadowing the SCSS
            // mixin) would otherwise leak through the bare-name path.
            if !sym.file_path.ends_with(".scss") {
                continue;
            }
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 0.85,
                strategy: "scss_bare_name",
                resolved_yield_type: None,
            });
        }
        None
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        // Property-value `call_expression` ref tagged by the extractor is
        // a CSS/SCSS built-in function evaluation. Always external, no
        // list required.
        if let Some(module) = &ref_ctx.extracted_ref.module {
            if module == super::extract::SCSS_CSS_FN_HINT {
                return Some("css".to_string());
            }
        }

        // @use / @forward of a Sass built-in module — the path itself is the
        // external namespace (e.g. "sass:math", "sass:color").
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let path = ref_ctx
                .extracted_ref
                .module
                .as_deref()
                .unwrap_or(target.as_str());
            if predicates::is_sass_builtin_module(path) {
                return Some(path.to_string());
            }
        }

        // Module-qualified access where the module is a Sass built-in.
        if let Some(module) = &ref_ctx.extracted_ref.module {
            if predicates::is_sass_builtin_module(module) {
                return Some(format!("sass:{module}"));
            }
            // Also check if an import entry for this module is a sass:* path.
            for import in &file_ctx.imports {
                if let Some(mp) = &import.module_path {
                    if (import.imported_name == *module || mp.contains(module.as_str()))
                        && predicates::is_sass_builtin_module(mp)
                    {
                        return Some(mp.clone());
                    }
                }
            }
        }

        engine::infer_external_common(file_ctx, ref_ctx, project_ctx, predicates::is_scss_builtin)
    }
}
