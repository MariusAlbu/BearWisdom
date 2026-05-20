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
        // Angular-template-only fallback: PascalCase component-selector
        // synthesized from a kebab-case HTML tag classifies against an
        // imported bare-specifier (`@org/foo` or `angular`-named) package.
        let target = &ref_ctx.extracted_ref.target_name;
        let is_component_selector_ref = ref_ctx.extracted_ref.kind == EdgeKind::Calls
            && target
                .chars()
                .next()
                .map_or(false, |c| c.is_ascii_uppercase())
            && !target.contains('.');
        if !is_component_selector_ref {
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
            if let Some(raw_selector) = &ref_ctx.extracted_ref.module {
                if let Some(class_qname) = lookup.angular_selector(raw_selector) {
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

pub static ANGULAR_HOOKS: AngularHooks = AngularHooks;
