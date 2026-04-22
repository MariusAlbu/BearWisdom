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
fn paired_ts_for_template(file_path: &str) -> Option<String> {
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
        TypeScriptResolver.infer_external_namespace_with_lookup(
            file_ctx, ref_ctx, project_ctx, lookup,
        )
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paired_ts_for_component_template() {
        assert_eq!(
            paired_ts_for_template("src/app/foo.component.html").as_deref(),
            Some("src/app/foo.component.ts")
        );
    }

    #[test]
    fn paired_ts_for_container_template() {
        assert_eq!(
            paired_ts_for_template("src/app/bar.container.html").as_deref(),
            Some("src/app/bar.container.ts")
        );
    }

    #[test]
    fn paired_ts_for_dialog_template() {
        assert_eq!(
            paired_ts_for_template("src/app/baz.dialog.html").as_deref(),
            Some("src/app/baz.dialog.ts")
        );
    }

    #[test]
    fn paired_ts_returns_none_for_plain_html() {
        assert_eq!(paired_ts_for_template("index.html"), None);
        assert_eq!(paired_ts_for_template("src/app/foo.component.ts"), None);
    }

    #[test]
    fn companion_file_for_imports_delegates_to_paired_ts() {
        let r = AngularResolver;
        assert_eq!(
            r.companion_file_for_imports("src/app/foo.component.html").as_deref(),
            Some("src/app/foo.component.ts")
        );
        assert_eq!(
            r.companion_file_for_imports("src/app/unrelated.html"),
            None
        );
    }
}
