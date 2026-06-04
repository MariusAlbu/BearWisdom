// R language hooks. Absorbed from the deleted `r_lang/resolve.rs`.

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, RefContext, SymbolLookup,
};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct RHooks;

pub(crate) fn detect_flow_inner(
    _file_ctx: &FileContext,
    ref_ctx: &RefContext,
) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, DbQueryOp, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;
    let r = &ref_ctx.extracted_ref;
    if r.kind != EdgeKind::Calls {
        return Vec::new();
    }
    let module = r.module.as_deref().unwrap_or("");
    let target = r.target_name.as_str();
    if (module == "httr" || module == "httr2")
        && matches!(
            target,
            "GET" | "POST" | "PUT" | "DELETE" | "HEAD" | "PATCH" | "request"
        )
    {
        let url = r.call_args.iter().find_map(|a| match a {
            CallArg::StringLit(s)
                if s.starts_with('/')
                    || s.starts_with("http://")
                    || s.starts_with("https://") =>
            {
                Some(s.as_str())
            }
            _ => None,
        });
        if let Some(url) = url {
            return vec![FlowEmission::NamedChannel {
                kind: NamedChannelKind::HttpCall,
                name: crate::connectors::url_pattern::normalize(url),
                role: ChannelRole::Producer,
                method: Some(HttpMethod::Any),
                streaming: None,
            }];
        }
    }
    if matches!(target, "dbGetQuery" | "dbSendQuery" | "dbExecute") {
        let sql = r.call_args.iter().find_map(|a| match a {
            CallArg::StringLit(s) => Some(s.as_str()),
            _ => None,
        });
        if let Some(sql) = sql {
            let upper = sql.to_ascii_uppercase();
            let op = if upper.contains("INSERT INTO") {
                DbQueryOp::Insert
            } else if upper.contains("UPDATE ") {
                DbQueryOp::Update
            } else if upper.contains("DELETE FROM") {
                DbQueryOp::Delete
            } else {
                DbQueryOp::Select
            };
            return vec![FlowEmission::DbQuery {
                entity_name: "r.*".to_string(),
                operation: op,
            }];
        }
    }
    Vec::new()
}

impl LanguageEngineHooks for RHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        _project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            return Some(target.clone());
        }
        if let Some(module) = &ref_ctx.extracted_ref.module {
            if file_ctx
                .imports
                .iter()
                .any(|i| i.module_path.as_deref() == Some(module.as_str()))
            {
                return Some(module.clone());
            }
        }
        None
    }

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
        project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        let mut imports = Vec::new();
        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(r.target_name.clone()),
                alias: None,
                is_wildcard: true,
            });
        }
        if let Some(ctx) = project_ctx {
            let all_deps = ctx.all_dependency_names();
            for dep in &all_deps {
                if matches!(
                    dep.as_str(),
                    "methods"
                        | "utils"
                        | "stats"
                        | "base"
                        | "datasets"
                        | "grDevices"
                        | "graphics"
                        | "tools"
                ) {
                    continue;
                }
                if !imports.iter().any(|i| i.module_path.as_deref() == Some(dep)) {
                    imports.push(ImportEntry {
                        imported_name: dep.clone(),
                        module_path: Some(dep.clone()),
                        alias: None,
                        is_wildcard: true,
                    });
                }
            }
        }
        Some(FileContext {
            file_path: file.path.clone(),
            language: "r".to_string(),
            imports,
            file_namespace: None,
        })
    }
}

pub static R_HOOKS: RHooks = RHooks;
