// Nix language hooks. Absorbed from the deleted `nix/resolve.rs`.

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, RefContext, Resolution, SymbolLookup,
};
use crate::type_checker::core::DefaultResolver;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct NixHooks;

impl LanguageEngineHooks for NixHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        _file_ctx: &FileContext,
        _project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let path = ref_ctx
                .extracted_ref
                .module
                .as_deref()
                .unwrap_or(target.as_str());
            if path.starts_with('<') && path.ends_with('>') {
                return Some(path.to_string());
            }
            return None;
        }
        if target.starts_with("builtins.")
            || target.starts_with("lib.")
            || target.starts_with("pkgs.")
            || target.starts_with("config.")
        {
            return Some("builtin".to_string());
        }
        None
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
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(module_path),
                alias: None,
                is_wildcard: false,
            });
        }
        Some(FileContext {
            file_path: file.path.clone(),
            language: "nix".to_string(),
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
        if target.starts_with("builtins.")
            || target.starts_with("lib.")
            || target.starts_with("pkgs.")
            || target.starts_with("config.")
        {
            return None;
        }
        if target.contains('.') {
            if let Some(sym) = lookup.by_qualified_name(target.as_str()) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "nix_qualified_name",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
            let last_seg = target.rsplit('.').next().unwrap_or(target.as_str());
            if let Some(sym) = lookup.by_name(last_seg).first() {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.75,
                    strategy: "nix_attr_path_last_seg",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }
        (DefaultResolver {
            file_ctx,
            ref_ctx,
            lookup,
            kind_compatible: |_, _| true,
        })
        .resolve_all()
    }
}

pub static NIX_HOOKS: NixHooks = NixHooks;
