// =============================================================================
// languages/gleam/resolve.rs — Gleam resolution rules
//
// Gleam uses a simple module system:
//
//   import gleam/list           → Imports, target_name = "list",  module = "gleam/list"
//   import myapp/utils          → Imports, target_name = "utils", module = "myapp/utils"
//   list.map(xs, f)             → Calls,   target_name = "map",   module = None
//   local_function()            → Calls,   target_name = "local_function", module = None
//
// The extractor strips the module qualifier from call sites (field_access nodes
// emit only the function name). So "list.map" becomes target_name = "map".
//
// Resolution strategy:
//   1. Same-file: functions defined in the same file are always in scope.
//   2. Import-based: for each imported module, try `{last_segment}.{target}`
//      as a qualified name, then a bare name lookup within the module.
//   3. Name-only fallback: global by_name lookup with lower confidence.
// =============================================================================

use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, RefContext, Resolution,
    SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

pub struct GleamResolver;

impl GleamResolver {

    
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

        // Skip import declarations — they declare scope, not symbol references.
        if edge_kind == EdgeKind::Imports {
            return None;
        }

        // Skip Gleam built-in operators emitted from binary_expression.
        if is_gleam_operator(target) {
            return None;
        }

        // Language-specific: import-based resolution with module alias lookup.
        // Gleam qualified names are stored as `module.function` in the index.
        for import in &file_ctx.imports {
            let Some(full_path) = &import.module_path else {
                continue;
            };

            // The local alias is the last path segment (e.g., "list" for "gleam/list").
            let module_alias = import
                .alias
                .as_deref()
                .unwrap_or_else(|| full_path.rsplit('/').next().unwrap_or(full_path.as_str()));

            // Try qualified name: {module}.{target}
            let candidate = format!("{module_alias}.{target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "gleam_import_qualified",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }

            // Try bare name within the imported module's file.
            for sym in lookup.in_file(full_path) {
                if sym.name == *target {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "gleam_import_file",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }

        engine::resolve_common("gleam", file_ctx, ref_ctx, lookup, |_, _| true)
    }

}

pub(crate) fn detect_gleam_http_producer(
    module: &str,
    target: &str,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;
    if !module.contains("httpc") && !module.contains("gleam/http") {
        return None;
    }
    let method = match target {
        "send" => HttpMethod::Any,
        "get" => HttpMethod::Get,
        "post" => HttpMethod::Post,
        _ => return None,
    };
    let url = call_args.iter().find_map(|a| match a {
        CallArg::StringLit(s)
            if s.starts_with('/') || s.starts_with("http://") || s.starts_with("https://") =>
        {
            Some(s.as_str())
        }
        _ => None,
    });
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name: url
            .map(crate::connectors::url_pattern::normalize)
            .unwrap_or_else(|| "*".to_string()),
        role: ChannelRole::Producer,
        method: Some(method),
    streaming: None,
    })
}

pub(crate) fn detect_gleam_pgo_emission(
    module: &str,
    target: &str,
    _call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    if !module.contains("pgo") && !module.contains("pog") && !module.contains("sqlight") {
        return None;
    }
    let op = match target {
        "execute" | "query" => DbQueryOp::Other,
        _ => return None,
    };
    Some(FlowEmission::DbQuery {
        entity_name: "gleam.*".to_string(),
        operation: op,
    })
}

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod tests;

/// Gleam binary operators emitted by the extractor as Calls refs.
/// These are language-level operators — not project symbols.
fn is_gleam_operator(name: &str) -> bool {
    matches!(
        name,
        "+" | "-" | "*" | "/" | "%" | "==" | "!=" | "<" | "<=" | ">" | ">="
            | "&&" | "||" | "!" | "|>" | "<>" | "+." | "-." | "*." | "/."
            | "==." | "!=." | "<." | "<=." | ">." | ">=."
    )
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
    if let Some(em) = detect_gleam_http_producer(module, target, &r.call_args) {
        return vec![em];
    }
    if let Some(em) = detect_gleam_pgo_emission(module, target, &r.call_args) {
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
        let module_path = r.module.clone().unwrap_or_else(|| r.target_name.clone());
        imports.push(ImportEntry {
            imported_name: r.target_name.clone(),
            module_path: Some(module_path),
            alias: None,
            // Gleam module imports bring qualified access into scope.
            // Mark as wildcard so the import walk can classify unresolved
            // bare names from external modules.
            is_wildcard: true,
        });
    }

    FileContext {
        file_path: file.path.clone(),
        language: "gleam".to_string(),
        imports,
        file_namespace: None,
    }
}
