// GDScript language hooks. Absorbed from the deleted `gdscript/resolve.rs`.

use super::predicates;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, RefContext, Resolution, SymbolLookup,
};
use crate::type_checker::core::DefaultResolver;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct GdScriptHooks;

impl LanguageEngineHooks for GdScriptHooks {
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
            language: "gdscript".to_string(),
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
        if matches!(target.as_str(), "super" | "$" | "if") {
            return None;
        }
        if !target.contains('.') && !target.contains('/') {
            let mut synthetic_match = None;
            let mut internal_match = None;
            for sym in lookup.by_name(target) {
                if !predicates::kind_compatible(edge_kind, &sym.kind) {
                    continue;
                }
                if sym.file_path.starts_with("ext:") {
                    synthetic_match = Some(sym);
                    break;
                } else if internal_match.is_none() {
                    internal_match = Some(sym);
                }
            }
            if let Some(sym) = synthetic_match.or(internal_match) {
                let strategy = if sym.file_path.starts_with("ext:") {
                    "gdscript_synthetic_global"
                } else {
                    "gdscript_internal_global"
                };
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: if strategy == "gdscript_synthetic_global" {
                        0.95
                    } else {
                        0.9
                    },
                    strategy,
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }
        (DefaultResolver {
            file_ctx,
            ref_ctx,
            lookup,
            kind_compatible: predicates::kind_compatible,
        })
        .resolve_all()
    }
}

pub static GDSCRIPT_HOOKS: GdScriptHooks = GdScriptHooks;
