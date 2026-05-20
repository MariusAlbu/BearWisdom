// =============================================================================
// haskell/resolve.rs — Haskell resolution rules
//
// Scope rules for Haskell:
//
//   1. Scope chain walk: innermost where/let → top-level.
//   2. Same-file resolution: all top-level bindings in the module are visible.
//   3. Import-based resolution:
//        `import Module`                  → wildcard import
//        `import qualified Module as M`   → aliased qualified import
//        `import Module (sym1, sym2)`     → selective import
//        `import Module hiding (sym)`     → hiding (treated as wildcard here)
//
// Haskell import model:
//   target_name = the module name (or alias when `as` is used)
//   module      = the original module name when an alias is present
// =============================================================================

use super::predicates;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution,
    SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// Haskell language resolver.
pub struct HaskellResolver;

impl LanguageResolver for HaskellResolver {
    fn language_ids(&self) -> &[&str] {
        &["haskell"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        let mut imports = Vec::new();

        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            // For Haskell, target_name is the module name or alias.
            // module is the original module name when an alias is present.
            let module_name = r.module.as_deref().unwrap_or(&r.target_name);
            let alias = if r.module.is_some() && r.target_name != module_name {
                Some(r.target_name.clone())
            } else {
                None
            };

            imports.push(ImportEntry {
                imported_name: module_name.to_string(),
                module_path: Some(module_name.to_string()),
                alias,
                is_wildcard: true,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "haskell".to_string(),
            imports,
            file_namespace: None,
        }
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

        // Bare-name walker lookup. cabal walks Hackage source jars when the
        // project's *.cabal declares a dep; Prelude / base / containers /
        // text symbols emit under ext:cabal:base/... Skip when chain
        // context is present.
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
                    strategy: "haskell_synthetic_global",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }

        engine::resolve_common("haskell", file_ctx, ref_ctx, lookup, predicates::kind_compatible)
    }


}

pub(crate) fn detect_haskell_scotty_route(
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
        "patch" => HttpMethod::Patch,
        "delete" => HttpMethod::Delete,
        "matchAny" => HttpMethod::Any,
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

pub(crate) fn detect_haskell_http_producer(
    module: &str,
    target: &str,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;
    let is_http_lib = module.contains("Network.HTTP")
        || module.contains("Network.Wreq")
        || module.contains("Network.HTTP.Req")
        || module.contains("Network.HTTP.Client");
    if !is_http_lib {
        return None;
    }
    if !matches!(target, "httpLbs" | "httpJSON" | "get" | "post" | "head" | "put" | "delete") {
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
        method: Some(HttpMethod::Any),
    streaming: None,
    })
}

pub(crate) fn detect_haskell_persistent_emission(
    module: &str,
    target: &str,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    if !module.contains("Database.Persist") && !module.contains("Database.Esqueleto") {
        return None;
    }
    let op = match target {
        "selectList" | "selectFirst" | "get" | "getBy" | "selectKeys" | "count" => {
            DbQueryOp::Select
        }
        "insert" | "insert_" | "insertEntity" | "insertMany" => DbQueryOp::Insert,
        "update" | "updateGet" | "replace" | "repsert" => DbQueryOp::Update,
        "delete" | "deleteWhere" => DbQueryOp::Delete,
        _ => return None,
    };
    Some(FlowEmission::DbQuery {
        entity_name: "hs.*".to_string(),
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
    // Scotty `get "/x" ...`, `post "/x" ...` — bare calls.
    if let Some(em) = detect_haskell_scotty_route(target, &r.call_args) {
        return vec![em];
    }
    if let Some(em) = detect_haskell_http_producer(module, target, &r.call_args) {
        return vec![em];
    }
    if let Some(em) = detect_haskell_persistent_emission(module, target) {
        return vec![em];
    }
    Vec::new()
}
