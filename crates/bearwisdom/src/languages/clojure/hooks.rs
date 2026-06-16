// Clojure language hooks. Absorbed from the deleted `clojure/resolve.rs`.

use super::predicates;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::legacy::{FileContext, ImportEntry, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct ClojureHooks;

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
    if !matches!(m_last, "client" | "http")
        || !(module.contains("clj-http")
            || module.contains("http-kit")
            || module.contains("org.httpkit"))
    {
        if !matches!(
            module,
            "clj-http.client" | "org.httpkit.client" | "hato.client"
        ) {
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
        "execute!" | "execute-one!" | "query" | "find-by-keys" | "get-by-id" => DbQueryOp::Select,
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

impl LanguageEngineHooks for ClojureHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        _project_ctx: Option<&ProjectContext>,
        lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;
        if predicates::is_java_interop(target) {
            return Some("java".to_string());
        }
        if predicates::is_java_class_ref(target) {
            return Some("java".to_string());
        }
        let is_camel = target.starts_with(|c: char| c.is_uppercase())
            && !target.contains('-')
            && !target.contains('/');
        if is_camel {
            let has_java_import = file_ctx.imports.iter().any(|imp| {
                imp.module_path
                    .as_deref()
                    .map(predicates::is_java_class_ref)
                    .unwrap_or(false)
            });
            if has_java_import {
                return Some("java".to_string());
            }
        }
        // A `:refer`-injected name whose namespace is not indexed declines the
        // bare-name import rung; classify it external to its source namespace so
        // it doesn't surface as a bare miss. The injected name is the entry's
        // `imported_name` (non-wildcard, module set).
        if let Some(import) = file_ctx.imports.iter().find(|i| {
            !i.is_wildcard && i.imported_name == *target && i.module_path.is_some()
        }) {
            if let Some(ns) = import.module_path.as_deref() {
                return Some(ns.to_string());
            }
        }
        if target.is_empty() || target.contains('/') || target.starts_with(':') {
            return None;
        }
        let has_internal_match = lookup
            .by_name(target)
            .iter()
            .any(|s| !s.file_path.starts_with("ext:"));
        if has_internal_match {
            return None;
        }
        let wildcard_ns = file_ctx.imports.iter().find(|i| {
            i.is_wildcard
                && i.module_path
                    .as_deref()
                    .map(|p| p.contains('.'))
                    .unwrap_or(false)
        })?;
        let ns = wildcard_ns.module_path.as_deref()?;
        let root = ns.split('.').next().unwrap_or(ns);
        Some(root.to_string())
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
            match r.module.as_deref() {
                // A `:refer`-injected name: the extractor keyed the namespace on
                // `module` and the bare injected name on `target_name`. This is an
                // ordinary import binding — `(:require [clojure.test :refer [is]])`
                // means `is` is in bare-name scope, sourced from `clojure.test`.
                // The injected name is the `imported_name` so the bare-name import
                // rung binds a call site `(is ...)` to the namespace's symbol,
                // never to a same-named function in an unrelated namespace.
                Some(ns) if ns != r.target_name => {
                    imports.push(ImportEntry {
                        imported_name: r.target_name.clone(),
                        module_path: Some(ns.to_string()),
                        alias: None,
                        is_wildcard: false,
                    });
                }
                // A whole-namespace import (`:require`/`:use`/`:import` entry with
                // no per-name `:refer`): the namespace is the wildcard scope.
                _ => {
                    let ns = r.module.as_deref().unwrap_or(&r.target_name);
                    imports.push(ImportEntry {
                        imported_name: ns.to_string(),
                        module_path: Some(ns.to_string()),
                        alias: None,
                        is_wildcard: true,
                    });
                }
            }
        }
        Some(FileContext {
            file_path: file.path.clone(),
            language: "clojure".to_string(),
            imports,
            file_namespace: None,
        })
    }
}

pub static CLOJURE_HOOKS: ClojureHooks = ClojureHooks;
