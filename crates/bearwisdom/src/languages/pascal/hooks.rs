// Pascal/Delphi language hooks. Absorbed from the deleted `pascal/resolve.rs`.

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, ImportEntry, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct PascalHooks;

pub(crate) fn is_delphi_namespaced_file(file_ctx: &FileContext) -> bool {
    const DELPHI_PREFIXES: &[&str] = &[
        "vcl.",
        "winapi.",
        "firedac.",
        "data.",
        "fmx.",
        "xml.",
        "system.generics.",
        "system.classes",
        "system.sysutils",
        "system.win.",
        "system.ioutils",
        "system.dateutils",
        "system.contnrs",
        "system.strutils",
        "system.variants",
        "system.math",
        "system.types",
        "system.uriparser",
    ];
    file_ctx.imports.iter().any(|imp| {
        imp.module_path.as_deref().map_or(false, |m| {
            let ml = m.to_lowercase();
            DELPHI_PREFIXES.iter().any(|p| ml.starts_with(p))
        })
    })
}

pub(crate) fn detect_pascal_http_producer(
    target: &str,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;
    let method = match target {
        "Get" => HttpMethod::Get,
        "Post" => HttpMethod::Post,
        "Put" => HttpMethod::Put,
        "Patch" => HttpMethod::Patch,
        "Delete" => HttpMethod::Delete,
        "Head" => HttpMethod::Head,
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

pub(crate) fn detect_pascal_db_query(
    target: &str,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use crate::types::CallArg;
    if !matches!(target, "ExecSQL" | "Open" | "Execute" | "Query") {
        return None;
    }
    let sql = call_args.iter().find_map(|a| match a {
        CallArg::StringLit(s) => Some(s.as_str()),
        _ => None,
    })?;
    let upper = sql.to_ascii_uppercase();
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
        entity_name: "pas.*".to_string(),
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
    let target = r.target_name.as_str();
    if let Some(em) = detect_pascal_http_producer(target, &r.call_args) {
        return vec![em];
    }
    if let Some(em) = detect_pascal_db_query(target, &r.call_args) {
        return vec![em];
    }
    Vec::new()
}

impl LanguageEngineHooks for PascalHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        _project_ctx: Option<&ProjectContext>,
        lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;
        let target_lower = target.to_lowercase();
        let keywords = super::keywords::KEYWORDS;
        if keywords.iter().any(|k| k.to_lowercase() == target_lower) {
            return Some("primitive".to_string());
        }
        let target_upper = target.to_uppercase();
        let target_title: String = {
            let mut c = target.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + &target_lower[f.len_utf8()..],
            }
        };
        for probe in [
            target.as_str(),
            target_lower.as_str(),
            target_title.as_str(),
            target_upper.as_str(),
        ] {
            for sym in lookup.by_name(probe) {
                if sym.file_path.starts_with("ext:pascal:")
                    && sym.name.to_lowercase() == target_lower
                {
                    return Some("fpc-runtime".to_string());
                }
            }
        }
        if is_delphi_namespaced_file(file_ctx) {
            return Some("delphi-vcl".to_string());
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
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(r.target_name.clone()),
                alias: None,
                is_wildcard: true,
            });
        }
        Some(FileContext {
            file_path: file.path.clone(),
            language: "pascal".to_string(),
            imports,
            file_namespace: None,
        })
    }
}

pub static PASCAL_HOOKS: PascalHooks = PascalHooks;
