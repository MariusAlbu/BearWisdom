// =============================================================================
// clojure/resolve.rs — Clojure resolution rules
//
// Scope rules for Clojure:
//
//   1. Scope chain walk: innermost let/letfn → defn → ns.
//   2. Same-file resolution: all top-level vars/defs in the namespace are visible.
//   3. Import-based resolution:
//        `(ns my.ns (:require [lib :as l]))` → aliased require
//        `(require '[lib :as l])`            → aliased require
//        `(use 'lib)`                        → wildcard use
//        `(import '(java.util Date))`        → Java class import
//
// Clojure import model:
//   target_name = the local alias or namespace name
//   module      = the canonical namespace when an alias is present
// =============================================================================

use super::predicates;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, RefContext, Resolution,
    SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// Clojure language resolver.
pub struct ClojureResolver;

impl ClojureResolver {

    
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
        let target = &ref_ctx.extracted_ref.target_name;
        let edge_kind = ref_ctx.extracted_ref.kind;

        if edge_kind == EdgeKind::Imports {
            return None;
        }

        engine::resolve_common("clojure", file_ctx, ref_ctx, lookup, predicates::kind_compatible)
    }

}

pub(crate) fn detect_clj_compojure_route(
    target_name: &str,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;
    let method = match target_name {
        "GET" => HttpMethod::Get,
        "POST" => HttpMethod::Post,
        "PUT" => HttpMethod::Put,
        "PATCH" => HttpMethod::Patch,
        "DELETE" => HttpMethod::Delete,
        "HEAD" => HttpMethod::Head,
        "OPTIONS" => HttpMethod::Options,
        "ANY" => HttpMethod::Any,
        _ => return None,
    };
    let url = call_args.iter().find_map(|a| match a {
        CallArg::StringLit(s) if s.starts_with('/') => Some(s.as_str()),
        _ => None,
    })?;
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name: crate::connectors::url_pattern::normalize(url),
        role: ChannelRole::Consumer,
        method: Some(method),
    streaming: None,
    })
}

pub(crate) fn detect_clj_http_producer(
    module: &str,
    target: &str,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;
    let m_last = module.rsplit('.').next().unwrap_or(module);
    if !matches!(m_last, "client" | "http") || !(module.contains("clj-http") || module.contains("http-kit") || module.contains("org.httpkit")) {
        // Accept clj-http.client and org.httpkit.client.
        if !matches!(module, "clj-http.client" | "org.httpkit.client" | "hato.client") {
            return None;
        }
    }
    let method = match target {
        "get" => HttpMethod::Get,
        "post" => HttpMethod::Post,
        "put" => HttpMethod::Put,
        "patch" => HttpMethod::Patch,
        "delete" => HttpMethod::Delete,
        "head" => HttpMethod::Head,
        _ => return None,
    };
    let url = call_args.iter().find_map(|a| match a {
        CallArg::StringLit(s)
            if s.starts_with('/') || s.starts_with("http://") || s.starts_with("https://") =>
        {
            Some(s.as_str())
        }
        _ => None,
    })?;
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name: crate::connectors::url_pattern::normalize(url),
        role: ChannelRole::Producer,
        method: Some(method),
    streaming: None,
    })
}

pub(crate) fn detect_clj_jdbc_db_query(
    module: &str,
    target: &str,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    if !module.contains("jdbc") && !module.contains("honeysql") {
        return None;
    }
    let op = match target {
        "execute!" | "execute-one!" | "query" | "find-by-keys" | "get-by-id" => {
            DbQueryOp::Select
        }
        "insert!" | "insert-multi!" => DbQueryOp::Insert,
        "update!" => DbQueryOp::Update,
        "delete!" => DbQueryOp::Delete,
        _ => return None,
    };
    Some(FlowEmission::DbQuery {
        entity_name: "clj.*".to_string(),
        operation: op,
    })
}

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod tests;

pub(crate) fn detect_flow_inner(
    _file_ctx: &FileContext,
    ref_ctx: &RefContext,
) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
    let r = &ref_ctx.extracted_ref;
    if r.kind != EdgeKind::Calls {
        return Vec::new();
    }
    let module = r.module.as_deref().unwrap_or("");
    let target = r.target_name.as_str();
    if let Some(em) = detect_clj_compojure_route(target, &r.call_args) {
        return vec![em];
    }
    if let Some(em) = detect_clj_http_producer(module, target, &r.call_args) {
        return vec![em];
    }
    if let Some(em) = detect_clj_jdbc_db_query(module, target) {
        return vec![em];
    }
    Vec::new()
}

pub(crate) fn build_file_context_inner(
    file: &ParsedFile,
    _project_ctx: Option<&ProjectContext>,
) -> FileContext {
    let mut imports = Vec::new();

    for r in &file.refs {
        if r.kind != EdgeKind::Imports {
            continue;
        }
        // target_name is the local alias or the full namespace.
        // module is the canonical namespace when an alias is present.
        let ns = r.module.as_deref().unwrap_or(&r.target_name);
        let alias = if r.module.is_some() && r.target_name != ns {
            Some(r.target_name.clone())
        } else {
            None
        };

        let is_wildcard = alias.is_none();
        imports.push(ImportEntry {
            imported_name: ns.to_string(),
            module_path: Some(ns.to_string()),
            alias,
            is_wildcard,
        });
    }

    FileContext {
        file_path: file.path.clone(),
        language: "clojure".to_string(),
        imports,
        file_namespace: None,
    }
}
