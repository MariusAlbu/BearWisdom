// Hare language hooks. Absorbed from the deleted `hare/resolve.rs`.

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, RefContext, Resolution, SymbolLookup,
};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct HareHooks;

pub(crate) fn is_hare_primitive(name: &str) -> bool {
    matches!(
        name,
        "bool" | "void" | "never" | "null" | "opaque"
            | "int" | "i8" | "i16" | "i32" | "i64"
            | "uint" | "u8" | "u16" | "u32" | "u64"
            | "uintptr" | "size" | "f32" | "f64"
            | "rune" | "str" | "bytes"
    )
}

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
    let module = r.module.as_deref().unwrap_or("");
    if !module.contains("http") {
        return Vec::new();
    }
    let target = r.target_name.as_str();
    if !matches!(target, "get" | "post" | "request" | "send") {
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

impl LanguageEngineHooks for HareHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        engine::infer_external_common(file_ctx, ref_ctx, project_ctx, is_hare_primitive)
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
            let local_name = module_path
                .rsplit("::")
                .next()
                .unwrap_or(module_path.as_str())
                .to_string();
            imports.push(ImportEntry {
                imported_name: local_name,
                module_path: Some(module_path),
                alias: None,
                is_wildcard: false,
            });
        }
        Some(FileContext {
            file_path: file.path.clone(),
            language: "hare".to_string(),
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
        if edge_kind == EdgeKind::Imports {
            return None;
        }
        if is_hare_primitive(target) {
            return None;
        }
        for import in &file_ctx.imports {
            let Some(mod_path) = &import.module_path else {
                continue;
            };
            let candidate = format!("{mod_path}::{target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "hare_import_qualified",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
            let candidate2 = format!("{}::{}", import.imported_name, target);
            if let Some(sym) = lookup.by_qualified_name(&candidate2) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "hare_import_local",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }
        engine::resolve_common("hare", file_ctx, ref_ctx, lookup, |_, _| true)
    }
}

pub static HARE_HOOKS: HareHooks = HareHooks;
