// SCSS language hooks. Absorbed from the deleted `scss/resolve.rs`.

use super::predicates;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, RefContext, Resolution, SymbolLookup,
};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct ScssHooks;

impl LanguageEngineHooks for ScssHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        _project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;
        if let Some(module) = &ref_ctx.extracted_ref.module {
            if module == super::extract::SCSS_CSS_FN_HINT {
                return Some("css".to_string());
            }
        }
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
        for import in &file_ctx.imports {
            let alias_matches = import.alias.as_deref() == Some(target.as_str())
                || import.imported_name == *target;
            if !alias_matches {
                continue;
            }
            if let Some(mp) = &import.module_path {
                if !mp.starts_with('.') && !mp.starts_with('/') {
                    let pkg = mp.split('/').next().unwrap_or(mp.as_str());
                    return Some(pkg.to_string());
                }
            }
        }
        None
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        let mut imports = Vec::new();
        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            let module_path = r.module.clone().unwrap_or_else(|| r.target_name.clone());
            let bare_segment = module_path
                .rsplit('/')
                .next()
                .unwrap_or(module_path.as_str())
                .trim_start_matches('_')
                .trim_end_matches(".scss")
                .trim_end_matches(".sass")
                .trim_end_matches(".css");
            let alias = if r.target_name != bare_segment {
                Some(r.target_name.clone())
            } else {
                None
            };
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(module_path),
                alias,
                is_wildcard: false,
            });
        }
        Some(FileContext {
            file_path: file.path.clone(),
            language: "scss".to_string(),
            imports,
            file_namespace: None,
        })
    }

    fn resolve_ref(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        let target = &ref_ctx.extracted_ref.target_name;
        let edge_kind = ref_ctx.extracted_ref.kind;
        if edge_kind == EdgeKind::Imports {
            return None;
        }
        if let Some(module) = &ref_ctx.extracted_ref.module {
            if module == super::extract::SCSS_CSS_FN_HINT {
                return None;
            }
        }
        if let Some(module) = &ref_ctx.extracted_ref.module {
            if predicates::is_sass_builtin_module(module) {
                return None;
            }
        }
        if let Some(res) = engine::resolve_common(
            "scss",
            file_ctx,
            ref_ctx,
            lookup,
            predicates::kind_compatible,
        ) {
            return Some(res);
        }
        if ref_ctx.extracted_ref.module.is_some() {
            return None;
        }
        let is_alias = file_ctx.imports.iter().any(|imp| {
            imp.alias.as_deref() == Some(target.as_str()) || imp.imported_name == *target
        });
        if is_alias {
            return None;
        }
        for sym in lookup.by_name(target) {
            if !predicates::kind_compatible(edge_kind, &sym.kind) {
                continue;
            }
            if !sym.file_path.ends_with(".scss")
                && !sym.file_path.ends_with(".sass")
                && !sym.file_path.ends_with(".css")
            {
                continue;
            }
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 0.85,
                strategy: "scss_bare_name",
                resolved_yield_type: None,
                flow_emit: None,
            });
        }
        None
    }
}

pub static SCSS_HOOKS: ScssHooks = ScssHooks;
