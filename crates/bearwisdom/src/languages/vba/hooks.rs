// VBA language hooks. Absorbed from the deleted `vba/resolve.rs`.

use super::predicates;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, RefContext, Resolution, SymbolLookup,
};
use crate::type_checker::core::DefaultResolver;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct VbaHooks;

impl LanguageEngineHooks for VbaHooks {
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
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: r.module.clone(),
                alias: None,
                is_wildcard: false,
            });
        }
        Some(FileContext {
            file_path: file.path.clone(),
            language: "vba".to_string(),
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
        if let Some(r) =
            (DefaultResolver {
            file_ctx,
            ref_ctx,
            lookup,
            kind_compatible: predicates::kind_compatible,
        })
        .resolve_all()
        {
            return Some(r);
        }
        // VBA identifiers are case-insensitive — case-folded same-file scan.
        let target_upper = target.to_uppercase();
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name.to_uppercase() == target_upper
                && predicates::kind_compatible(edge_kind, &sym.kind)
            {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.95,
                    strategy: "vba_case_insensitive",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }
        None
    }
}

pub static VBA_HOOKS: VbaHooks = VbaHooks;
