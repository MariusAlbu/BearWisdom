// =============================================================================
// languages/angular/resolve.rs — Angular-specific LanguageResolver
//
// Wraps the TypeScript resolver. Angular templates (`.component.html`,
// `.container.html`, `.dialog.html`) carry no import statements of their own,
// so the generic TypeScript resolver's `file_ctx.imports` would be empty for
// them. But every symbol a template references is imported by the paired
// `.component.ts` (or `.container.ts` / `.dialog.ts`) class that declares the
// template. So this resolver's entire job is to route Angular template files
// through TypeScriptResolver with the companion `.ts` file's imports merged
// in — the driver does the merge via `companion_file_for_imports`, and
// everything else falls through to standard TS resolution (imports →
// chain walk → external classification → heuristic).
//
// Result: `<c-col>` in the template resolves to `@coreui/angular.CCol`
// exactly the way `CCol` would resolve in the `.component.ts` file itself —
// no naming-pattern heuristic, no per-library hardcoding, and any future
// Angular component library (Material, PrimeNG, anything) gets the same
// treatment automatically.
//
// Selector-map path (PR 18):
// When the template extractor emits a `Calls` ref for a kebab-case tag
// (e.g. `<app-user-card>`), it stores the raw tag in `ref_ctx.extracted_ref.module`
// and the PascalCase fallback in `target_name`. `resolve()` here first checks the
// project-wide Angular selector map (`lookup.angular_selector(raw)`) built from
// `@Component({selector:'...'})` metadata. When a match is found the real class
// qname is used for DB lookup — exact, no heuristic. When no match exists the
// call falls through to the TypeScriptResolver (which eventually hits the
// external-namespace heuristic for third-party component libraries).
// =============================================================================

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    FileContext, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::languages::typescript::resolve::TypeScriptResolver;
use crate::types::ParsedFile;

pub struct AngularResolver;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// The paired TypeScript file for an Angular template, or `None` for
/// non-template files.
///
/// Angular pairs `<name>.component.html` with `<name>.component.ts` (and the
/// same for `.container` / `.dialog`). Returning the path lets the resolve
/// driver fetch that file's imports and merge them into the template's
/// `FileContext`.
pub(crate) fn paired_ts_for_template(file_path: &str) -> Option<String> {
    const SUFFIXES: &[&str] = &[".component.html", ".container.html", ".dialog.html"];
    for suffix in SUFFIXES {
        if let Some(stem) = file_path.strip_suffix(suffix) {
            let ts_suffix = suffix.trim_end_matches(".html").to_string() + ".ts";
            return Some(format!("{stem}{ts_suffix}"));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// LanguageResolver impl
// ---------------------------------------------------------------------------

impl LanguageResolver for AngularResolver {
    fn language_ids(&self) -> &[&str] {
        // Only claim the Angular language IDs — `typescript` and `javascript`
        // remain owned by `TypeScriptResolver` so Angular-specific behavior
        // does not affect non-Angular projects. Component `.ts` files are
        // detected as "typescript" and handled by `TypeScriptResolver`
        // directly; only template files reach this resolver.
        &["angular", "angular_template"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        TypeScriptResolver.build_file_context(file, project_ctx)
    }

    fn companion_file_for_imports(&self, file_path: &str) -> Option<String> {
        paired_ts_for_template(file_path)
    }

    fn resolve(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        // Angular selector-map path (PR 18):
        // The template extractor stores the raw kebab/attribute selector in
        // `ref_ctx.extracted_ref.module` for Calls refs emitted from template
        // tags. Check the project-wide selector map first — an exact hit
        // replaces the PascalCase heuristic guess with the real class qname.
        if ref_ctx.extracted_ref.kind == crate::types::EdgeKind::Calls {
            if let Some(raw_selector) = &ref_ctx.extracted_ref.module {
                if let Some(class_qname) = lookup.angular_selector(raw_selector) {
                    // The selector matched a @Component class in this project.
                    // Try to resolve to the real DB symbol.
                    if let Some(sym) = lookup.by_qualified_name(class_qname) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 1.0,
                            strategy: "angular_selector_map",
                            resolved_yield_type: None,
                            flow_emit: None,
                        });
                    }
                    // The class is indexed by qname — also try by_name as fallback.
                    let short = class_qname.rsplit('.').next().unwrap_or(class_qname);
                    for sym in lookup.by_name(short) {
                        if sym.qualified_name == class_qname {
                            return Some(Resolution {
                                target_symbol_id: sym.id,
                                confidence: 1.0,
                                strategy: "angular_selector_map",
                                resolved_yield_type: None,
                                flow_emit: None,
                            });
                        }
                    }
                }
            }
        }

        TypeScriptResolver.resolve(file_ctx, ref_ctx, lookup)
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        TypeScriptResolver.infer_external_namespace(file_ctx, ref_ctx, project_ctx)
    }

    fn infer_external_namespace_with_lookup(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
        lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        if let Some(ns) = TypeScriptResolver.infer_external_namespace_with_lookup(
            file_ctx, ref_ctx, project_ctx, lookup,
        ) {
            return Some(ns);
        }
        // Angular-template-only fallback: PascalCase component-selector
        // synthesized from a kebab-case HTML tag (e.g. `<lucide-icon>` →
        // target `LucideIcon`, `<router-outlet>` → `RouterOutlet`). These
        // never match a symbol name directly because the real class lives
        // under a package qname (`@angular/router.RouterOutletImpl`,
        // `lucide-angular.LucideIcon`) AND the component's kebab selector
        // metadata isn't extracted from `@Component({selector: ...})`. When
        // the ref is a PascalCase Calls in a template and ANY companion
        // import is a bare-specifier, classify against that package — the
        // template is rendering a component from one of the imported
        // Angular modules, whichever one provides the selector.
        let target = &ref_ctx.extracted_ref.target_name;
        let is_component_selector_ref = ref_ctx.extracted_ref.kind == crate::types::EdgeKind::Calls
            && target
                .chars()
                .next()
                .map_or(false, |c| c.is_ascii_uppercase())
            && !target.contains('.');
        if !is_component_selector_ref {
            return None;
        }
        // Prefer imports whose first segment looks like a component-oriented
        // Angular package (heuristic: scoped `@org/foo` or a name containing
        // `angular`). Those are the modules that declare selectors.
        let mut fallback: Option<String> = None;
        for import in &file_ctx.imports {
            let Some(module) = import.module_path.as_deref() else {
                continue;
            };
            if !crate::languages::typescript::predicates::is_bare_specifier(module) {
                continue;
            }
            if fallback.is_none() {
                fallback = Some(module.to_string());
            }
            if module.starts_with('@') || module.contains("angular") {
                return Some(module.to_string());
            }
        }
        fallback
    }
}

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod tests;
