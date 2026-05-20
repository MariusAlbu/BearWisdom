// =============================================================================
// indexer/resolve/rules/dart/mod.rs — Dart resolution rules
//
// Scope rules for Dart:
//
//   1. Scope chain walk: innermost class/function → outermost.
//   2. Same-file resolution: all top-level symbols are visible within the file.
//   3. Import-based resolution: `import 'package:foo/bar.dart'` or
//      `import '../models/user.dart'` brings symbols into scope.
//   4. Fully qualified names via `as` prefix aliases.
//
// Dart import model:
//   `import 'dart:core'`                   → stdlib (always in scope)
//   `import 'package:flutter/material.dart'` → external pub package
//   `import '../models/user.dart'`          → project-relative
//   `import 'user.dart' as u`              → aliased import
//
// The extractor emits EdgeKind::Imports with:
//   target_name = the URI string or the local alias
//   module      = the URI string (for `as` imports)
// =============================================================================


use super::predicates;
use crate::ecosystem::manifest::ManifestKind;
use crate::type_checker::chain::{
    self, ChainConfig, NamespaceLookup, identity_normalize,
};
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// Dart language resolver.
pub struct DartResolver;

impl LanguageResolver for DartResolver {
    fn language_ids(&self) -> &[&str] {
        &["dart"]
    }

    
    fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        build_file_context_inner(file, project_ctx)
    }
fn resolve(
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

        // Bare-name walker lookup. dart_sdk + flutter_sdk + pub emit real
        // symbols for dart:core (String, List, Map, Future, Stream),
        // dart:async, dart:io, Flutter widgets, and declared pub deps.
        // ext:-only filter, gated on chain.is_none().
        if ref_ctx.extracted_ref.chain.is_none() && !target.contains('.') {
            for sym in lookup.by_name(target) {
                if !sym.file_path.starts_with("ext:") {
                    continue;
                }
                if !predicates::kind_compatible(edge_kind, &sym.kind) {
                    continue;
                }
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.95,
                    strategy: "dart_synthetic_global",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }

        // Chain-aware resolution: walk MemberChain following field/return types.
        // Dart has generics but no wildcard-import namespace lookup.
        if let Some(chain_val) = &ref_ctx.extracted_ref.chain {
            let config = ChainConfig {
                strategy_prefix: "dart",
                normalize_type: identity_normalize,
                has_self_ref: true,
                enclosing_type_kinds: &["class", "enum", "mixin"],
                static_type_kinds: &["class", "enum", "mixin", "type_alias", "extension"],
                use_generics: true,
                namespace_lookup: NamespaceLookup::None,
                kind_compatible: predicates::kind_compatible,
            };
            if let Some(res) = chain::resolve_via_chain(
                &config, chain_val, edge_kind, Some(file_ctx), ref_ctx, lookup,
            ) {
                return Some(res);
            }
        }

        let effective_target = target.strip_prefix("this.").unwrap_or(target);

        // Step 1: Scope chain walk.
        for scope in &ref_ctx.scope_chain {
            let candidate = format!("{scope}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "dart_scope_chain",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }

        // Step 2: Same-file resolution.
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name == effective_target && predicates::kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "dart_same_file",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }

        // Step 3: Simple name lookup across the project.
        for sym in lookup.by_name(effective_target) {
            if predicates::kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.85,
                    strategy: "dart_by_name",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }

        // Dart bare-name fallback. Continues the cross-language
        // template (PRs 31, 35-40, plus Lua, Go, Rust, Kotlin, Ruby).
        // Dart's `import 'package:foo/bar.dart'` brings whole
        // libraries into scope; bare-name calls fall here when the
        // engine's module/import path can't bind. Gated by `.dart`
        // file extension.
        if matches!(edge_kind, EdgeKind::Calls | EdgeKind::TypeRef | EdgeKind::Instantiates)
            && ref_ctx.extracted_ref.module.is_none()
            && !target.contains('.')
        {
            for sym in lookup.by_name(target) {
                if !predicates::kind_compatible(edge_kind, &sym.kind) {
                    continue;
                }
                let path = &sym.file_path;
                let is_dart = path.ends_with(".dart");
                if !is_dart {
                    continue;
                }
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.80,
                    strategy: "dart_bare_name",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }

        None
    }

}

pub(crate) fn detect_dart_shelf_route(
    target_name: &str,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;
    // Router pattern: `router.get('/x', handler)` shows up as chain — handled elsewhere.
    // Shelf top-level Router DSL bare-calls.
    let method = match target_name {
        "get" => HttpMethod::Get,
        "post" => HttpMethod::Post,
        "put" => HttpMethod::Put,
        "patch" => HttpMethod::Patch,
        "delete" => HttpMethod::Delete,
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

pub(crate) fn detect_dart_http_chain(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;
    if chain.segments.len() < 2 {
        return None;
    }
    let leaf = chain.segments.last()?.name.as_str();
    let method = match leaf {
        "get" => HttpMethod::Get,
        "post" => HttpMethod::Post,
        "put" => HttpMethod::Put,
        "patch" => HttpMethod::Patch,
        "delete" => HttpMethod::Delete,
        _ => return None,
    };
    let root = chain.segments[0].name.as_str();
    // Dio / http.Client / dart:io HttpClient — Producer.
    // Restrict to common client identifiers to avoid hijacking shelf's
    // `router.get(...)` Consumer path.
    if !matches!(root, "dio" | "http" | "client" | "_client" | "Dio") {
        return None;
    }
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

pub(crate) fn detect_dart_drift_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    if chain.segments.len() < 2 {
        return None;
    }
    // Drift `select(users).get()` / `update(users).write(...)`.
    let root = chain.segments[0].name.as_str();
    let leaf = chain.segments.last()?.name.as_str();
    let op = match root {
        "select" => DbQueryOp::Select,
        "insert" => DbQueryOp::Insert,
        "update" => DbQueryOp::Update,
        "delete" => DbQueryOp::Delete,
        _ => return None,
    };
    if !matches!(
        leaf,
        "get" | "getSingle" | "watch" | "write" | "insert" | "go" | "go!" | "do"
    ) {
        return None;
    }
    Some(FlowEmission::DbQuery {
        entity_name: "dart.*".to_string(),
        operation: op,
    })
}

pub(crate) fn detect_dart_grpc_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    if chain.segments.len() < 2 {
        return None;
    }
    let root = chain.segments[0].name.as_str();
    if !root.ends_with("Client") || root == "Client" {
        return None;
    }
    let leaf = chain.segments.last()?.name.as_str();
    if matches!(leaf, "shutdown" | "terminate") {
        return None;
    }
    let service = root.strip_suffix("Client").unwrap_or(root);
    use crate::indexer::resolve::flow_emit::StreamKind;
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::RpcCall,
        name: format!("{}.{}", service, leaf),
        role: ChannelRole::Producer,
        method: None,
        streaming: Some(StreamKind::from_method_name(leaf)),
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
    if let Some(chain) = r.chain.as_ref() {
        if let Some(em) = detect_dart_http_chain(chain, &r.call_args) {
            return vec![em];
        }
        if let Some(em) = detect_dart_drift_emission(chain) {
            return vec![em];
        }
        if let Some(em) = detect_dart_grpc_emission(chain) {
            return vec![em];
        }
    }
    if let Some(em) = detect_dart_shelf_route(r.target_name.as_str(), &r.call_args) {
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
        // For Dart, target_name is the import URI (or alias for `as` imports).
        // module is the URI when an alias is present.
        let uri = r.module.as_deref().unwrap_or(&r.target_name);
        let alias = if r.module.is_some() && r.target_name != uri {
            Some(r.target_name.clone())
        } else {
            None
        };

        imports.push(ImportEntry {
            imported_name: uri.to_string(),
            module_path: Some(uri.to_string()),
            alias,
            is_wildcard: false,
        });
    }

    FileContext {
        file_path: file.path.clone(),
        language: "dart".to_string(),
        imports,
        file_namespace: None,
    }
}
