// Nim language hooks. Absorbed from the deleted `nim/resolve.rs`.

use super::predicates;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, RefContext, Resolution, SymbolInfo, SymbolLookup,
};
use crate::type_checker::core::DefaultResolver;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct NimHooks;

pub(crate) fn detect_nim_jester_route(
    target_name: &str,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;
    let method = match target_name {
        "get" => HttpMethod::Get,
        "post" => HttpMethod::Post,
        "put" => HttpMethod::Put,
        "delete" => HttpMethod::Delete,
        "patch" => HttpMethod::Patch,
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

pub(crate) fn detect_nim_http_producer(
    module: &str,
    target: &str,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;
    if !module.contains("httpclient") && !module.contains("http_client") {
        return None;
    }
    let method = match target {
        "get" | "getContent" => HttpMethod::Get,
        "post" | "postContent" => HttpMethod::Post,
        "put" => HttpMethod::Put,
        "delete" => HttpMethod::Delete,
        "request" => HttpMethod::Any,
        _ => return None,
    };
    let url = call_args.iter().find_map(|a| match a {
        CallArg::StringLit(s)
            if s.starts_with('/')
                || s.starts_with("http://")
                || s.starts_with("https://") =>
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

pub(crate) fn detect_nim_db_emission(
    module: &str,
    target: &str,
    _call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    if !module.contains("db_postgres")
        && !module.contains("db_sqlite")
        && !module.contains("db_mysql")
        && !module.contains("norm")
    {
        return None;
    }
    let op = match target {
        "exec" | "execAffectedRows" => DbQueryOp::Other,
        "getRow" | "getAllRows" | "rows" => DbQueryOp::Select,
        "tryExec" | "insertId" | "insertID" => DbQueryOp::Insert,
        _ => return None,
    };
    Some(FlowEmission::DbQuery {
        entity_name: "nim.*".to_string(),
        operation: op,
    })
}

fn nim_module_file_stem_resolve(
    file_ctx: &FileContext,
    target: &str,
    edge_kind: EdgeKind,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    let mut import_leaves: Vec<&str> = Vec::new();
    let mut import_packages: Vec<&str> = Vec::new();
    let mut has_stdlib_import = false;
    for imp in &file_ctx.imports {
        let mp = match imp.module_path.as_deref() {
            Some(mp) => mp,
            None => continue,
        };
        let is_std = mp.starts_with("std/")
            || matches!(
                mp,
                "system" | "os" | "strutils" | "sequtils" | "tables" | "sets"
                    | "math" | "options" | "json" | "times" | "algorithm" | "unicode"
                    | "streams" | "hashes" | "sugar" | "macros" | "parseutils"
                    | "strformat" | "pegs" | "re" | "uri" | "asyncdispatch"
                    | "asyncnet" | "httpclient" | "logging" | "terminal"
            );
        if is_std {
            has_stdlib_import = true;
        }
        let stripped = mp
            .strip_prefix("std/")
            .or_else(|| mp.strip_prefix("pkg/"))
            .unwrap_or(mp);
        let leaf = stripped.rsplit('/').next().unwrap_or(stripped);
        import_leaves.push(leaf);
        let pkg = mp.split('/').next().unwrap_or(mp);
        if pkg != "std" && pkg != "pkg" && !pkg.is_empty() {
            import_packages.push(pkg);
        }
    }
    if import_leaves.is_empty() {
        return None;
    }
    let by_name = lookup.by_name(target);
    let nim_external: Vec<&SymbolInfo> = by_name
        .iter()
        .filter(|s| {
            let fp = s.file_path.as_ref();
            (fp.starts_with("ext:nim:") || fp.starts_with("ext:idx:"))
                && fp.to_lowercase().ends_with(".nim")
        })
        .collect();
    if nim_external.is_empty() {
        return None;
    }
    for sym in &nim_external {
        if !predicates::kind_compatible(edge_kind, &sym.kind) {
            continue;
        }
        let fl = sym.file_path.to_lowercase().replace('\\', "/");
        for leaf in &import_leaves {
            let leaf_lower = leaf.to_lowercase();
            if fl.ends_with(&format!("/{leaf_lower}.nim"))
                || fl.contains(&format!("/{leaf_lower}/"))
            {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.85,
                    strategy: "nim_module_file_stem",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }
    }
    for sym in &nim_external {
        if !predicates::kind_compatible(edge_kind, &sym.kind) {
            continue;
        }
        let fl = sym.file_path.to_lowercase().replace('\\', "/");
        for pkg in &import_packages {
            let pkg_lower = pkg.to_lowercase();
            if fl.contains(&format!("/{pkg_lower}/")) || fl.contains(&format!(":{pkg_lower}/")) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.75,
                    strategy: "nim_package_level",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }
    }
    if has_stdlib_import {
        for sym in &nim_external {
            if !predicates::kind_compatible(edge_kind, &sym.kind) {
                continue;
            }
            let fl = sym.file_path.as_ref();
            if fl.contains("ext:nim:nim-stdlib/") {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.70,
                    strategy: "nim_stdlib_any",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }
    }
    None
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
    if let Some(em) = detect_nim_jester_route(target, &r.call_args) {
        return vec![em];
    }
    if let Some(em) = detect_nim_http_producer(module, target, &r.call_args) {
        return vec![em];
    }
    if let Some(em) = detect_nim_db_emission(module, target, &r.call_args) {
        return vec![em];
    }
    Vec::new()
}

impl LanguageEngineHooks for NimHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        _file_ctx: &FileContext,
        _project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;
        if ref_ctx.extracted_ref.kind != EdgeKind::Imports {
            return None;
        }
        if let Some(rest) = target.strip_prefix("std/") {
            return Some(format!("ext:nim-stdlib:{rest}"));
        }
        if target.starts_with("std/") {
            return Some("ext:nim-stdlib".to_string());
        }
        if let Some(rest) = target.strip_prefix("pkg/") {
            return Some(format!("ext:nim-pkg:{rest}"));
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
        imports.push(ImportEntry {
            imported_name: "system".to_string(),
            module_path: Some("system".to_string()),
            alias: None,
            is_wildcard: true,
        });
        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            let module_path = r.module.clone().unwrap_or_else(|| r.target_name.clone());
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(module_path),
                alias: None,
                is_wildcard: r.module.is_none(),
            });
        }
        Some(FileContext {
            file_path: file.path.clone(),
            language: "nim".to_string(),
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
        if let Some(res) = (DefaultResolver {
            file_ctx,
            ref_ctx,
            lookup,
            kind_compatible: predicates::kind_compatible,
        })
        .resolve_all() {
            return Some(res);
        }
        nim_module_file_stem_resolve(file_ctx, target, edge_kind, lookup)
    }
}

pub static NIM_HOOKS: NimHooks = NimHooks;
