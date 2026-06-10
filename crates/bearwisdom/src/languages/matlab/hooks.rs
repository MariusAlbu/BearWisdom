// MATLAB language hooks. Absorbed from the deleted `matlab/resolve.rs`.

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, ImportEntry, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct MatlabHooks;

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
    let target = r.target_name.as_str();
    if matches!(
        target,
        "webread" | "webwrite" | "urlread" | "urlwrite" | "websave"
    ) {
        let url = r.call_args.iter().find_map(|a| match a {
            CallArg::StringLit(s)
                if s.starts_with('/') || s.starts_with("http://") || s.starts_with("https://") =>
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
    if matches!(target, "fetch" | "exec") {
        let sql = r.call_args.iter().find_map(|a| match a {
            CallArg::StringLit(s) => Some(s.as_str()),
            _ => None,
        });
        if let Some(sql) = sql {
            let upper = sql.to_ascii_uppercase();
            if upper.contains("SELECT") || upper.contains("FROM ") {
                return vec![FlowEmission::DbQuery {
                    entity_name: "matlab.*".to_string(),
                    operation: DbQueryOp::Select,
                }];
            }
        }
    }
    Vec::new()
}

impl LanguageEngineHooks for MatlabHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        _file_ctx: &FileContext,
        _project_ctx: Option<&ProjectContext>,
        lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;
        let bare = target.split('.').next().unwrap_or(target);
        let hits = lookup.by_name(bare);
        if hits
            .iter()
            .any(|sym| sym.file_path.starts_with("ext:matlab:"))
        {
            return Some("matlab-runtime".to_string());
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
                is_wildcard: r.target_name.ends_with(".*"),
            });
        }
        Some(FileContext {
            file_path: file.path.clone(),
            language: "matlab".to_string(),
            imports,
            file_namespace: None,
        })
    }
}

pub static MATLAB_HOOKS: MatlabHooks = MatlabHooks;
