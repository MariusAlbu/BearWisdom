// =============================================================================
// matlab/resolve.rs — MATLAB resolution rules
//
// MATLAB module system:
//   - No explicit import statements in general code.
//   - `addpath('dir')` adds a directory to the search path at runtime.
//   - Each .m file defines one primary function or a classdef.
//   - Package directories are prefixed with `+`: `+mypackage/MyClass.m`.
//   - Within a classdef, methods reference other methods by name directly.
//
// Resolution strategy:
//   1. Scope chain walk (class method → class → file).
//   2. Same-file symbols (nested functions, local helpers).
//   3. Project-wide name lookup (each .m file is a callable unit).
// =============================================================================

use super::predicates;
use crate::indexer::resolve::engine::{
    self, FileContext, ImportEntry, RefContext, Resolution, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// MATLAB language resolver.
pub struct MatlabResolver;

impl MatlabResolver {

    
    pub(crate) fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        build_file_context_inner(file, project_ctx)
    }
pub(crate) fn resolve(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            return None;
        }

        engine::resolve_common("matlab", file_ctx, ref_ctx, lookup, predicates::kind_compatible)
    }


}

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod tests;

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
    // webread/webwrite/urlread Producer.
    if matches!(target, "webread" | "webwrite" | "urlread" | "urlwrite" | "websave") {
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
    // Database Toolbox: fetch / exec.
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

pub(crate) fn build_file_context_inner(
    file: &ParsedFile,
    _project_ctx: Option<&ProjectContext>,
) -> FileContext {
    let mut imports = Vec::new();

    // MATLAB uses addpath() rather than import directives, but the extractor
    // may emit EdgeKind::Imports for `import pkg.*` (OOP MATLAB). Collect
    // those here so downstream steps can use them.
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

    FileContext {
        file_path: file.path.clone(),
        language: "matlab".to_string(),
        imports,
        file_namespace: None,
    }
}
