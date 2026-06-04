// OCaml language hooks. Absorbed from the deleted `ocaml/resolve.rs`.

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, RefContext, SymbolLookup,
};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct OcamlHooks;

pub(crate) fn detect_ocaml_dream_route(
    module: &str,
    target_name: &str,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;
    let m_last = module.rsplit('.').next().unwrap_or(module);
    if !matches!(m_last, "Dream" | "App" | "Opium") {
        return None;
    }
    let method = match target_name {
        "get" => HttpMethod::Get,
        "post" => HttpMethod::Post,
        "put" => HttpMethod::Put,
        "delete" => HttpMethod::Delete,
        "patch" => HttpMethod::Patch,
        "head" => HttpMethod::Head,
        "options" => HttpMethod::Options,
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

pub(crate) fn detect_ocaml_cohttp_producer(
    module: &str,
    target_name: &str,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;
    if !module.contains("Cohttp") && !module.contains("Piaf") && !module.contains("Httpaf") {
        return None;
    }
    let method = match target_name {
        "get" | "call" => HttpMethod::Any,
        "post" => HttpMethod::Post,
        "put" => HttpMethod::Put,
        "delete" => HttpMethod::Delete,
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

pub(crate) fn detect_ocaml_caqti_emission(
    module: &str,
    target_name: &str,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    detect_ocaml_caqti_with_imports(module, target_name, &[])
}

pub(crate) fn detect_ocaml_caqti_with_imports(
    module: &str,
    target_name: &str,
    aliases: &[(String, String)],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let canonical_match = module.contains("Caqti")
        || matches!(module, "Db" | "Database" | "Repo" | "Q" | "Conn");
    let m_root = module.split('.').next().unwrap_or(module);
    let alias_match = aliases
        .iter()
        .any(|(local, target)| local == m_root && target.contains("Caqti"));
    if !canonical_match && !alias_match {
        return None;
    }
    let op = match target_name {
        "find" | "find_opt" | "collect_list" | "fold" | "iter" | "rev_collect_list" => {
            DbQueryOp::Select
        }
        "exec" => DbQueryOp::Other,
        _ => return None,
    };
    Some(FlowEmission::DbQuery {
        entity_name: "ml.*".to_string(),
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
    if let Some(em) = detect_ocaml_dream_route(module, target, &r.call_args) {
        return vec![em];
    }
    if let Some(em) = detect_ocaml_cohttp_producer(module, target, &r.call_args) {
        return vec![em];
    }
    if let Some(em) = detect_ocaml_caqti_emission(module, target) {
        return vec![em];
    }
    Vec::new()
}

impl LanguageEngineHooks for OcamlHooks {
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
        let mut imports = vec![ImportEntry {
            imported_name: "Stdlib".to_string(),
            module_path: Some("stdlib".to_string()),
            alias: None,
            is_wildcard: true,
        }];
        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            let source_module = r.module.as_deref().unwrap_or(&r.target_name);
            let alias = if r.module.is_some() && r.target_name != source_module {
                Some(r.target_name.clone())
            } else {
                None
            };
            imports.push(ImportEntry {
                imported_name: source_module.to_string(),
                module_path: Some(source_module.to_string()),
                alias,
                is_wildcard: true,
            });
        }
        Some(FileContext {
            file_path: file.path.clone(),
            language: "ocaml".to_string(),
            imports,
            file_namespace: None,
        })
    }
}

pub static OCAML_HOOKS: OcamlHooks = OcamlHooks;
