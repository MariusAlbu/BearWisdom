// OCaml language hooks. Absorbed from the deleted `ocaml/resolve.rs`.

use super::predicates;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, RefContext, Resolution, SymbolInfo, SymbolLookup,
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

    fn resolve_ref(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        let edge_kind = ref_ctx.extracted_ref.kind;
        if edge_kind == EdgeKind::Imports {
            return None;
        }
        if let Some(res) = engine::resolve_common(
            "ocaml",
            file_ctx,
            ref_ctx,
            lookup,
            predicates::kind_compatible,
        ) {
            return Some(res);
        }
        if let Some(module) = &ref_ctx.extracted_ref.module {
            let target = &ref_ctx.extracted_ref.target_name;
            if let Some(dot) = module.find('.') {
                let stripped_module = &module[dot + 1..];
                let candidate = format!("{stripped_module}.{target}");
                if let Some(sym) = lookup.by_qualified_name(&candidate) {
                    if predicates::kind_compatible(edge_kind, &sym.kind) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 0.90,
                            strategy: "ocaml_stem_stripped",
                            resolved_yield_type: None,
                            flow_emit: None,
                        });
                    }
                }
                let stripped_lower = stripped_module.to_lowercase();
                let last_seg = stripped_lower.rsplit('.').next().unwrap_or(&stripped_lower);
                let by_name = lookup.by_name(target);
                if let Some(sym) = by_name.iter().find(|s: &&SymbolInfo| {
                    let fl = s.file_path.to_lowercase().replace('\\', "/");
                    (fl.contains(&format!("/{last_seg}."))
                        || fl.contains(&format!("/{last_seg}/")))
                        && predicates::kind_compatible(edge_kind, &s.kind)
                }) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 0.88,
                        strategy: "ocaml_stem_stripped_name",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            } else {
                let module_lower = module.to_lowercase();
                let by_name = lookup.by_name(target);
                if let Some(sym) = by_name.iter().find(|s: &&SymbolInfo| {
                    let fl = s.file_path.to_lowercase().replace('\\', "/");
                    (fl.ends_with(&format!("/{module_lower}.ml"))
                        || fl.ends_with(&format!("/{module_lower}.mli"))
                        || fl.contains(&format!("/{module_lower}/")))
                        && predicates::kind_compatible(edge_kind, &s.kind)
                }) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 0.92,
                        strategy: "ocaml_module_to_file_stem",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }
        None
    }
}

pub static OCAML_HOOKS: OcamlHooks = OcamlHooks;
