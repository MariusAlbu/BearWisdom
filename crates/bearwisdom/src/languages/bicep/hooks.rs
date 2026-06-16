// Bicep language hooks. Absorbed from the deleted `bicep/resolve.rs`.

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::legacy::{
    self as engine, FileContext, ImportEntry, RefContext, SymbolLookup,
};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct BicepHooks;

pub(crate) fn is_azure_resource_type(name: &str) -> bool {
    let stripped = name.trim_matches('\'');
    if !stripped.contains('/') {
        return false;
    }
    let lower = stripped.to_ascii_lowercase();
    if lower.starts_with("br:") || lower.starts_with("br/") || lower.starts_with("az:") {
        return true;
    }
    let head = stripped.split('/').next().unwrap_or("");
    if head.is_empty() {
        return false;
    }
    head.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '$' | '{' | '}'))
}

pub(crate) fn is_child_resource_shorthand(name: &str) -> bool {
    if name.is_empty() || name.contains('/') {
        return false;
    }
    let base = name.split('@').next().unwrap_or(name);
    if base.is_empty() {
        return false;
    }
    if !base.chars().all(|c| c.is_ascii_alphanumeric()) {
        return false;
    }
    let starts_lower = base
        .chars()
        .next()
        .map(|c| c.is_ascii_lowercase())
        .unwrap_or(false);
    let all_upper = base.chars().all(|c| !c.is_ascii_lowercase());
    starts_lower || all_upper
}

impl LanguageEngineHooks for BicepHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;
        let edge_kind = ref_ctx.extracted_ref.kind;
        if is_azure_resource_type(target) {
            return Some("azure".to_string());
        }
        if edge_kind == EdgeKind::TypeRef && is_child_resource_shorthand(target) {
            return Some("azure".to_string());
        }
        engine::infer_external_common(file_ctx, ref_ctx, project_ctx, |_| false)
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
                module_path: r.module.clone().or_else(|| Some(r.target_name.clone())),
                alias: None,
                is_wildcard: false,
            });
        }
        Some(FileContext {
            file_path: file.path.clone(),
            language: "bicep".to_string(),
            imports,
            file_namespace: None,
        })
    }
}

pub static BICEP_HOOKS: BicepHooks = BicepHooks;
