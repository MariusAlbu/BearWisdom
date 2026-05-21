// Angular language hooks. Absorbed from the deleted `angular/resolve.rs`.

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    FileContext, RefContext, Resolution, SymbolLookup,
};
use crate::languages::typescript::hooks::TypeScriptResolver;
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
            file_ctx, ref_ctx, project_ctx, lookup,
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
        Some(TypeScriptResolver.build_file_context(file, project_ctx))
    }

    fn resolve_ref(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        if ref_ctx.extracted_ref.kind == EdgeKind::Calls {
            let target = &ref_ctx.extracted_ref.target_name;
            // The angular_selector map is keyed on the literal string from
            // @Component({selector:'...'}) / @Directive({selector:'...'}).
            // Two shapes need to match:
            //   * kebab-case component tags — `<app-user-card>` becomes
            //     target_name="AppUserCard" in the extractor; the map key
            //     is "app-user-card". Try the kebab-derived form.
            //   * camelCase attribute directives — `[appHighlight]` keeps
            //     target_name="appHighlight"; the map key is "appHighlight"
            //     (or its bracketed form, but the directive registry
            //     unbrackets it). Try the literal target first.
            let kebab = pascal_to_kebab(target);
            for candidate in [target.as_str(), kebab.as_str()] {
                if let Some(class_qname) = lookup.angular_selector(candidate) {
                    if let Some(sym) = lookup.by_qualified_name(class_qname) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 1.0,
                            strategy: "angular_selector_map",
                            resolved_yield_type: None,
                            flow_emit: None,
                        });
                    }
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
}

/// "AppAvatar" -> "app-avatar". Splits on every uppercase boundary and
/// lowercases segments. Single-segment inputs (already lowercase or
/// camelCase with no uppercase boundary) return unchanged.
fn pascal_to_kebab(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (i, ch) in name.chars().enumerate() {
        if ch.is_ascii_uppercase() && i > 0 {
            out.push('-');
        }
        out.extend(ch.to_lowercase());
    }
    out
}

pub static ANGULAR_HOOKS: AngularHooks = AngularHooks;
