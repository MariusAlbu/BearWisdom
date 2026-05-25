// Prolog language hooks. Absorbed from the deleted `prolog/resolve.rs`.

use super::predicates;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, RefContext, Resolution, SymbolInfo, SymbolLookup,
};
use crate::type_checker::core::DefaultResolver;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct PrologHooks;

fn is_prolog_runtime_path(path: &str) -> bool {
    let p = path.replace('\\', "/").to_ascii_lowercase();
    (p.contains("/library/") || p.contains("/boot/"))
        && (p.contains("swipl") || p.contains("swi-prolog") || p.contains("prolog"))
}

impl LanguageEngineHooks for PrologHooks {
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
            let module_path = r.module.clone().or_else(|| Some(r.target_name.clone()));
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path,
                alias: None,
                is_wildcard: true,
            });
        }
        Some(FileContext {
            file_path: file.path.clone(),
            language: "prolog".to_string(),
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
        let effective_target = target.split(':').last().unwrap_or(target);
        if effective_target != target.as_str() {
            for scope in &ref_ctx.scope_chain {
                let candidate = format!("{scope}.{effective_target}");
                if let Some(sym) = lookup.by_qualified_name(&candidate) {
                    if predicates::kind_compatible(edge_kind, &sym.kind) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 1.0,
                            strategy: "prolog_scope_chain",
                            resolved_yield_type: None,
                            flow_emit: None,
                        });
                    }
                }
            }
            for sym in lookup.in_file(&file_ctx.file_path) {
                if sym.name == effective_target
                    && predicates::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "prolog_same_file",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
            return None;
        }
        if let Some(res) = (DefaultResolver {
            file_ctx,
            ref_ctx,
            lookup,
            kind_compatible: predicates::kind_compatible,
        })
        .resolve_all() {
            return Some(res);
        }
        let mut runtime_hit: Option<&SymbolInfo> = None;
        let mut internal_hit: Option<&SymbolInfo> = None;
        for sym in lookup.by_name(target) {
            if !predicates::kind_compatible(edge_kind, &sym.kind) {
                continue;
            }
            if sym.file_path.starts_with("ext:") || is_prolog_runtime_path(&sym.file_path) {
                if runtime_hit.is_none() {
                    runtime_hit = Some(sym);
                }
            } else if internal_hit.is_none() {
                internal_hit = Some(sym);
            }
        }
        if let Some(sym) = runtime_hit {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 0.85,
                strategy: "prolog_runtime_fallback",
                resolved_yield_type: None,
                flow_emit: None,
            });
        }
        if let Some(sym) = internal_hit {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 0.80,
                strategy: "prolog_project_by_name",
                resolved_yield_type: None,
                flow_emit: None,
            });
        }
        None
    }
}

pub static PROLOG_HOOKS: PrologHooks = PrologHooks;
