// =============================================================================
// perl/resolve.rs — Perl resolution rules
//
// Scope rules for Perl:
//
//   1. Scope chain walk: innermost subroutine/package → outermost.
//   2. Same-file resolution: all subroutines in the file are visible.
//   3. By-name lookup: for used modules, symbols may be defined elsewhere.
//
// Perl import model:
//   `use Module;`       → target_name = "Module"
//   `use Module qw(…);` → target_name = "Module"
//   `require Module;`   → target_name = "Module"
//
// The extractor emits EdgeKind::Imports with target_name = the module name.
// =============================================================================

use super::predicates;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution,
    SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// Perl language resolver.
pub struct PerlResolver;

impl LanguageResolver for PerlResolver {
    fn language_ids(&self) -> &[&str] {
        &["perl"]
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

        // Bare-name walker lookup. perl_stdlib walks <perl_root>/lib/<ver>/
        // for core modules (Carp, Data::Dumper, File::Path, IO::File, ...).
        // Interpreter built-ins (print, chomp, map, ...) are handled by the
        // engine's primitive set populated from `keywords()` — they
        // classify as `"primitive"` namespace via classify_external_name.
        if !target.contains("::") {
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
                    strategy: "perl_synthetic_global",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }

        engine::resolve_common("perl", file_ctx, ref_ctx, lookup, predicates::kind_compatible)
    }


}

pub(crate) fn detect_perl_route(
    target_name: &str,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;
    // Dancer `get '/x' => sub { ... }`, `post '/x' => ...`.
    let method = match target_name {
        "get" => HttpMethod::Get,
        "post" => HttpMethod::Post,
        "put" => HttpMethod::Put,
        "patch" => HttpMethod::Patch,
        "del" | "delete" => HttpMethod::Delete,
        "any" => HttpMethod::Any,
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

pub(crate) fn detect_perl_http_producer(
    module: &str,
    target: &str,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;
    // LWP::UserAgent `$ua->get($url)`, Mojo::UserAgent `$ua->get(...)`.
    let m_last = module.rsplit("::").next().unwrap_or(module);
    if !matches!(m_last, "UserAgent" | "Tiny" | "Furl") {
        return None;
    }
    let method = match target {
        "get" => HttpMethod::Get,
        "post" => HttpMethod::Post,
        "put" => HttpMethod::Put,
        "delete" => HttpMethod::Delete,
        "head" => HttpMethod::Head,
        "patch" => HttpMethod::Patch,
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

pub(crate) fn detect_perl_dbi_emission(
    _module: &str,
    target: &str,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use crate::types::CallArg;
    if !matches!(
        target,
        "prepare" | "do" | "selectrow_array" | "selectrow_hashref" | "selectall_arrayref"
            | "selectall_hashref" | "selectcol_arrayref" | "execute"
    ) {
        return None;
    }
    let sql = call_args.iter().find_map(|a| match a {
        CallArg::StringLit(s) => Some(s.as_str()),
        _ => None,
    })?;
    let upper = sql.trim().to_ascii_uppercase();
    let op = if upper.contains("INSERT INTO") {
        DbQueryOp::Insert
    } else if upper.contains("UPDATE ") {
        DbQueryOp::Update
    } else if upper.contains("DELETE FROM") {
        DbQueryOp::Delete
    } else if upper.contains(" FROM ") || upper.starts_with("SELECT") {
        DbQueryOp::Select
    } else {
        return None;
    };
    Some(FlowEmission::DbQuery {
        entity_name: "pl.*".to_string(),
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
    if let Some(em) = detect_perl_route(target, &r.call_args) {
        return vec![em];
    }
    if let Some(em) = detect_perl_http_producer(module, target, &r.call_args) {
        return vec![em];
    }
    if let Some(em) = detect_perl_dbi_emission(module, target, &r.call_args) {
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
        imports.push(ImportEntry {
            imported_name: r.target_name.clone(),
            module_path: Some(r.target_name.clone()),
            alias: None,
            is_wildcard: false,
        });
    }

    FileContext {
        file_path: file.path.clone(),
        language: "perl".to_string(),
        imports,
        file_namespace: None,
    }
}
