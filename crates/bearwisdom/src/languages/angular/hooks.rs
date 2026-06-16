// Angular language hooks. Component/directive selector resolution is generic
// engine code driven by `ANGULAR_PROFILE.selector_resolution` (`resolve_via_selector_map`);
// the hook keeps only the external classifier (template-selector → imported
// bare-specifier fallback) and the import-table builder.

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::legacy::{FileContext, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct AngularHooks;

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

impl LanguageEngineHooks for AngularHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        if let Some(ns) = crate::languages::typescript::hooks::infer_external_inner_with_lookup(
            file_ctx,
            ref_ctx,
            project_ctx,
            lookup,
        ) {
            return Some(ns);
        }
        // Angular-template fallback: a template ref produced by the
        // template extractor (component tag or attribute directive)
        // classifies against an imported bare-specifier package when no
        // project-level @Component/@Directive matched. Both shapes count:
        //   * PascalCase component refs derived from kebab tags
        //     (`<app-card>` -> `AppCard`).
        //   * camelCase attribute-directive refs (`cButton`, `routerLink`,
        //     `ngFor`) — the @coreui/angular and @angular/router worlds
        //     use these heavily and they were previously dropped because
        //     the first-char-uppercase guard skipped them.
        let target = &ref_ctx.extracted_ref.target_name;
        let first_ch = target.chars().next();
        let starts_alpha = first_ch.map_or(false, |c| c.is_ascii_alphabetic());
        let has_upper = target.chars().any(|c| c.is_ascii_uppercase());
        let is_template_selector_ref = ref_ctx.extracted_ref.kind == EdgeKind::Calls
            && starts_alpha
            && (first_ch.map_or(false, |c| c.is_ascii_uppercase()) || has_upper)
            && !target.contains('.');
        if !is_template_selector_ref {
            return None;
        }
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

    fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        Some(crate::languages::typescript::hooks::build_file_context_inner(file, project_ctx))
    }
}

pub static ANGULAR_HOOKS: AngularHooks = AngularHooks;
