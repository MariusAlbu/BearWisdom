// Fortran language hooks. Absorbed from the deleted `fortran/resolve.rs`.

use super::predicates;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, RefContext, Resolution, SymbolLookup,
};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct FortranHooks;

pub(crate) fn detect_flow_inner(
    _file_ctx: &FileContext,
    ref_ctx: &RefContext,
) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;
    let r = &ref_ctx.extracted_ref;
    if r.kind != EdgeKind::Calls {
        return Vec::new();
    }
    let target = r.target_name.as_str().to_lowercase();
    if !matches!(target.as_str(), "curl_easy_setopt" | "curl_easy_perform") {
        return Vec::new();
    }
    let url = r.call_args.iter().find_map(|a| match a {
        CallArg::StringLit(s)
            if s.starts_with('/')
                || s.starts_with("http://")
                || s.starts_with("https://") =>
        {
            Some(s.as_str())
        }
        _ => None,
    });
    let Some(url) = url else { return Vec::new() };
    vec![FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name: crate::connectors::url_pattern::normalize(url),
        role: ChannelRole::Producer,
        method: Some(HttpMethod::Any),
        streaming: None,
    }]
}

impl LanguageEngineHooks for FortranHooks {
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
            let is_rename = !r.namespace_segments.is_empty();
            if is_rename {
                let module_path = r.namespace_segments.first().cloned();
                let source_name = r.module.clone().unwrap_or_default();
                let local_name = r.target_name.clone();
                if !source_name.is_empty() {
                    imports.push(ImportEntry {
                        imported_name: source_name,
                        module_path,
                        alias: Some(local_name),
                        is_wildcard: false,
                    });
                }
            } else if r.module.is_some() {
                imports.push(ImportEntry {
                    imported_name: r.target_name.clone(),
                    module_path: r.module.clone(),
                    alias: None,
                    is_wildcard: false,
                });
            } else {
                imports.push(ImportEntry {
                    imported_name: r.target_name.clone(),
                    module_path: Some(r.target_name.clone()),
                    alias: None,
                    is_wildcard: true,
                });
            }
        }
        Some(FileContext {
            file_path: file.path.clone(),
            language: "fortran".to_string(),
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
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            return None;
        }
        let target = &ref_ctx.extracted_ref.target_name;
        let target_lower = target.to_lowercase();
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name.to_lowercase() == target_lower
                && predicates::kind_compatible(ref_ctx.extracted_ref.kind, &sym.kind)
            {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "fortran_same_file",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }
        if let Some(type_name) = &ref_ctx.extracted_ref.module {
            let type_lower = type_name.to_lowercase();
            for tname in [type_name.as_str(), type_lower.as_str()] {
                for member in lookup.members_of(tname) {
                    if member.name.to_lowercase() == target_lower {
                        for sym in lookup.by_name(target) {
                            if sym.name.to_lowercase() == target_lower
                                && predicates::kind_compatible(
                                    ref_ctx.extracted_ref.kind,
                                    &sym.kind,
                                )
                            {
                                return Some(Resolution {
                                    target_symbol_id: sym.id,
                                    confidence: 0.9,
                                    strategy: "fortran_type_member",
                                    resolved_yield_type: None,
                                    flow_emit: None,
                                });
                            }
                        }
                        return Some(Resolution {
                            target_symbol_id: member.id,
                            confidence: 0.85,
                            strategy: "fortran_type_member_direct",
                            resolved_yield_type: None,
                            flow_emit: None,
                        });
                    }
                }
            }
        }
        engine::resolve_common(
            "fortran",
            file_ctx,
            ref_ctx,
            lookup,
            predicates::kind_compatible,
        )
    }
}

pub static FORTRAN_HOOKS: FortranHooks = FortranHooks;
