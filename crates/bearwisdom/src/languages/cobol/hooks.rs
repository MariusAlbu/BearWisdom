// COBOL language hooks. Absorbed from the deleted `cobol/resolve.rs`.

use super::predicates;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, RefContext, Resolution, SymbolLookup,
};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct CobolHooks;

pub(crate) fn detect_flow_inner(
    _file_ctx: &FileContext,
    ref_ctx: &RefContext,
) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use crate::types::CallArg;
    let r = &ref_ctx.extracted_ref;
    if r.kind != EdgeKind::Calls {
        return Vec::new();
    }
    let sql = r.call_args.iter().find_map(|a| match a {
        CallArg::StringLit(s) => Some(s.as_str()),
        _ => None,
    });
    let Some(sql) = sql else { return Vec::new() };
    let upper = sql.to_ascii_uppercase();
    if !upper.contains("SELECT")
        && !upper.contains("INSERT")
        && !upper.contains("UPDATE")
        && !upper.contains("DELETE")
    {
        return Vec::new();
    }
    let op = if upper.contains("INSERT INTO") {
        DbQueryOp::Insert
    } else if upper.contains("UPDATE ") {
        DbQueryOp::Update
    } else if upper.contains("DELETE FROM") {
        DbQueryOp::Delete
    } else {
        DbQueryOp::Select
    };
    vec![FlowEmission::DbQuery {
        entity_name: "cobol.*".to_string(),
        operation: op,
    }]
}

impl LanguageEngineHooks for CobolHooks {
    fn detect_flow_emissions(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        _lookup: &dyn SymbolLookup,
    ) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
        detect_flow_inner(file_ctx, ref_ctx)
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
            language: "cobol".to_string(),
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
        let edge_kind = ref_ctx.extracted_ref.kind;
        if edge_kind == EdgeKind::Imports {
            return None;
        }
        engine::resolve_common(
            "cobol",
            file_ctx,
            ref_ctx,
            lookup,
            predicates::kind_compatible,
        )
    }
}

pub static COBOL_HOOKS: CobolHooks = CobolHooks;
