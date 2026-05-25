// Zig language hooks. Absorbed from the deleted `zig/resolve.rs`.

use super::predicates;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, RefContext, Resolution, SymbolLookup,
};
use crate::type_checker::core::DefaultResolver;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct ZigHooks;

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
    let target = r.target_name.as_str();
    if matches!(target, "fetch" | "send" | "open") {
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
        if let Some(url) = url {
            return vec![FlowEmission::NamedChannel {
                kind: NamedChannelKind::HttpCall,
                name: crate::connectors::url_pattern::normalize(url),
                role: ChannelRole::Producer,
                method: Some(HttpMethod::Any),
                streaming: None,
            }];
        }
    }
    Vec::new()
}

impl LanguageEngineHooks for ZigHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        if let Some(ns) = engine::infer_external_common(
            file_ctx,
            ref_ctx,
            project_ctx,
            predicates::is_zig_builtin,
        ) {
            return Some(if ns == "builtin" {
                "zig.builtin".to_string()
            } else {
                ns
            });
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
            let module_path = r.module.clone().unwrap_or_else(|| r.target_name.clone());
            let alias = if r.module.is_some() {
                Some(r.target_name.clone())
            } else {
                None
            };
            imports.push(ImportEntry {
                imported_name: module_path.clone(),
                module_path: Some(module_path),
                alias,
                is_wildcard: false,
            });
        }
        Some(FileContext {
            file_path: file.path.clone(),
            language: "zig".to_string(),
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
        if predicates::is_zig_builtin(target) {
            return None;
        }
        (DefaultResolver {
            file_ctx,
            ref_ctx,
            lookup,
            kind_compatible: predicates::kind_compatible,
        })
        .resolve_all()
    }
}

pub static ZIG_HOOKS: ZigHooks = ZigHooks;
