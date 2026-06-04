// Fortran language hooks. Absorbed from the deleted `fortran/resolve.rs`.

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, RefContext, SymbolLookup,
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
}

pub static FORTRAN_HOOKS: FortranHooks = FortranHooks;
