// =============================================================================
// languages/angular/resolve.rs — Angular-specific LanguageResolver
//
// Wraps the TypeScript resolver and adds heuristics for Angular template
// files (`.component.html`, `.container.html`, `.dialog.html`).
//
// Angular templates have no TypeScript import statements, so the generic
// TypeScript resolver's `infer_external_namespace` has no import context to
// work with. This resolver adds Angular-specific classification:
//
//   1. CoreUI Angular components — `c-*` selectors are converted to `CXxx`
//      by the template extractor.  When `@coreui/angular` (or sibling CoreUI
//      packages) is listed as an npm dependency, classify unresolved `C[A-Z]...`
//      targets as belonging to that package.
//
//   2. Angular structural directives — `*ngIf` / `*ngFor` → `NgIfDirective` /
//      `NgForDirective`.  Classify these as `@angular/common`.
//
//   3. Delegation — all other resolution (imports from the TS extractor,
//      scope-chain, same-file) falls through to TypeScriptResolver.
// =============================================================================

use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    FileContext, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::languages::typescript::resolve::TypeScriptResolver;
use crate::types::ParsedFile;

/// Angular template and component resolver.
///
/// Delegates to `TypeScriptResolver` for all TS-based files (`.ts`, `.tsx`)
/// and standard resolution paths. For template files the override only
/// activates in `infer_external_namespace_with_lookup`, which fires after the
/// main resolver returned `None` — guaranteeing the target is genuinely absent
/// from the project index before we classify it as external.
pub struct AngularResolver;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// `true` when the file path is an Angular template (HTML host file).
fn is_angular_template(file_path: &str) -> bool {
    file_path.ends_with(".component.html")
        || file_path.ends_with(".container.html")
        || file_path.ends_with(".dialog.html")
}

/// `true` when `name` matches the CoreUI Angular component naming pattern:
/// starts with an uppercase `C` followed by another uppercase letter.
///
/// CoreUI Angular selectors follow `c-row`, `c-card`, `c-col`, … and the
/// template extractor converts them to `CRow`, `CCard`, `CCol`, … via
/// `kebab_to_pascal`.  The `CC` pattern (two initial capitals) reliably
/// identifies these without hardcoding the class names.
fn is_coreui_component_name(name: &str) -> bool {
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some('C'), Some(second)) => second.is_ascii_uppercase(),
        _ => false,
    }
}

/// `true` when the target name looks like an Angular structural directive
/// class synthesised by the template extractor's `*ngXxx` expansion:
/// `NgIfDirective`, `NgForDirective`, `NgSwitchDirective`, etc.
fn is_angular_directive_name(name: &str) -> bool {
    name.starts_with("Ng") && name.ends_with("Directive")
}

/// Find the first `@coreui/` npm package listed in the project's manifest
/// that is plausibly the source for `component_name`.
///
/// - If the name starts with `CChart` → prefer `@coreui/angular-chartjs`.
/// - Otherwise → prefer `@coreui/angular`.
/// - Returns the first matching package found in the dependency set, or `None`
///   when no `@coreui/` package is declared at all.
fn coreui_package_for(
    component_name: &str,
    npm_deps: &std::collections::HashSet<String>,
) -> Option<String> {
    // Prefer the chartjs sub-package for chart components.
    let preferred = if component_name.starts_with("CChart") {
        "@coreui/angular-chartjs"
    } else {
        "@coreui/angular"
    };

    if npm_deps.contains(preferred) {
        return Some(preferred.to_string());
    }

    // Fall back to any @coreui/ dep present in the manifest.
    npm_deps
        .iter()
        .find(|dep| dep.starts_with("@coreui/"))
        .cloned()
}

// ---------------------------------------------------------------------------
// LanguageResolver impl
// ---------------------------------------------------------------------------

impl LanguageResolver for AngularResolver {
    fn language_ids(&self) -> &[&str] {
        // Only claim the Angular language IDs — `typescript` and `javascript`
        // remain owned by `TypeScriptResolver` so Angular-specific heuristics
        // do not shadow TS resolution for non-Angular projects.
        // Angular component `.ts` files are tagged as "typescript" by the file
        // detector and handled by `TypeScriptResolver`; only the HTML template
        // files are tagged "angular" or "angular_template" and reach this resolver.
        &["angular", "angular_template"]
    }

    fn build_file_context(&self, file: &ParsedFile, project_ctx: Option<&ProjectContext>) -> FileContext {
        TypeScriptResolver.build_file_context(file, project_ctx)
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
        // Angular-specific heuristics only apply inside template files.
        if is_angular_template(&file_ctx.file_path) {
            let target = &ref_ctx.extracted_ref.target_name;

            // Angular structural directives (NgIfDirective, NgForDirective …)
            if is_angular_directive_name(target) {
                return Some("@angular/common".to_string());
            }

            // CoreUI component pattern — only when the npm dep is present.
            if is_coreui_component_name(target) {
                if let Some(ctx) = project_ctx {
                    if let Some(npm) = ctx
                        .manifests_for(ref_ctx.file_package_id)
                        .get(&ManifestKind::Npm)
                    {
                        if let Some(pkg) = coreui_package_for(target, &npm.dependencies) {
                            return Some(pkg);
                        }
                    }
                }
            }
        }

        // Delegate everything else to the TypeScript resolver.
        TypeScriptResolver.infer_external_namespace(file_ctx, ref_ctx, project_ctx)
    }

    fn infer_external_namespace_with_lookup(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
        lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        // Try the Angular-specific (lookup-free) path first.
        if let Some(ns) = self.infer_external_namespace(file_ctx, ref_ctx, project_ctx) {
            return Some(ns);
        }
        // Delegate to the TypeScript resolver's barrel/re-export inspection.
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
    fn is_coreui_component_name_matches_coreui_pattern() {
        assert!(is_coreui_component_name("CCol"));
        assert!(is_coreui_component_name("CCard"));
        assert!(is_coreui_component_name("CCardBody"));
        assert!(is_coreui_component_name("CRow"));
        assert!(is_coreui_component_name("CProgress"));
        assert!(is_coreui_component_name("CChart"));
    }

    #[test]
    fn is_coreui_component_name_rejects_non_matches() {
        // Project-local components (app-xxx → AppXxx) start with 'A', not 'C'.
        assert!(!is_coreui_component_name("AppDocsExample"));
        // Standard TS/Angular class names that happen to start with 'C'.
        assert!(!is_coreui_component_name("Component")); // C + lowercase 'o'
        assert!(!is_coreui_component_name("CardService")); // C + lowercase 'a'
        // Too short.
        assert!(!is_coreui_component_name("C"));
        assert!(!is_coreui_component_name(""));
    }

    #[test]
    fn is_angular_template_matches_expected_extensions() {
        assert!(is_angular_template("foo.component.html"));
        assert!(is_angular_template("bar.container.html"));
        assert!(is_angular_template("baz.dialog.html"));
        assert!(!is_angular_template("foo.html"));
        assert!(!is_angular_template("foo.component.ts"));
    }

    #[test]
    fn is_angular_directive_name_matches_ng_directives() {
        assert!(is_angular_directive_name("NgIfDirective"));
        assert!(is_angular_directive_name("NgForDirective"));
        assert!(is_angular_directive_name("NgSwitchDirective"));
        assert!(!is_angular_directive_name("NgModule")); // no "Directive" suffix
        assert!(!is_angular_directive_name("MyDirective")); // no "Ng" prefix
    }

    #[test]
    fn coreui_package_for_selects_angular_by_default() {
        let mut deps = std::collections::HashSet::new();
        deps.insert("@coreui/angular".to_string());
        deps.insert("@coreui/angular-chartjs".to_string());

        assert_eq!(
            coreui_package_for("CCol", &deps),
            Some("@coreui/angular".to_string())
        );
        assert_eq!(
            coreui_package_for("CChart", &deps),
            Some("@coreui/angular-chartjs".to_string())
        );
    }

    #[test]
    fn coreui_package_for_returns_none_when_not_in_deps() {
        let deps = std::collections::HashSet::new();
        assert_eq!(coreui_package_for("CCol", &deps), None);
    }
}
