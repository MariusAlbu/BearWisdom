// HEEx language hooks. Absorbed from the deleted `heex/resolve.rs`.

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, RefContext, Resolution, SymbolLookup,
};
use crate::languages::elixir;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct HeexHooks;

impl LanguageEngineHooks for HeexHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        _file_ctx: &FileContext,
        _project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;
        if target.contains('.') {
            let root = target.split('.').next().unwrap_or(target);
            if elixir::predicates::is_external_elixir_module(root) {
                return Some(root.to_string());
            }
        }
        None
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        Some(FileContext {
            file_path: file.path.clone(),
            language: "heex".to_string(),
            imports: Vec::<ImportEntry>::new(),
            file_namespace: None,
        })
    }

    fn resolve_ref(
        &self,
        _file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        let target = &ref_ctx.extracted_ref.target_name;
        let edge_kind = ref_ctx.extracted_ref.kind;
        if edge_kind != EdgeKind::Calls {
            return None;
        }
        if target.contains('.') {
            return None;
        }
        for sym in lookup.by_name(target) {
            if !sym.file_path.starts_with("ext:") {
                continue;
            }
            if !elixir::predicates::kind_compatible(edge_kind, &sym.kind) {
                continue;
            }
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 0.90,
                strategy: "heex_ext_component",
                resolved_yield_type: None,
                flow_emit: None,
            });
        }
        for sym in lookup.by_name(target) {
            if sym.file_path.starts_with("ext:") {
                continue;
            }
            if !elixir::predicates::kind_compatible(edge_kind, &sym.kind) {
                continue;
            }
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 0.80,
                strategy: "heex_internal_component",
                resolved_yield_type: None,
                flow_emit: None,
            });
        }
        None
    }
}

pub static HEEX_HOOKS: HeexHooks = HeexHooks;
