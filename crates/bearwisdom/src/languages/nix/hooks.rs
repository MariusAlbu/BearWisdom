// Nix language hooks. Absorbed from the deleted `nix/resolve.rs`.

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, RefContext, SymbolLookup,
};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct NixHooks;

/// Reserved Nix namespace prefixes — `builtins`, `lib`, `pkgs`, `config` —
/// whose dotted members (`builtins.toString`, `lib.mkOption`) are language /
/// nixpkgs builtins, not project symbols.
pub(crate) fn is_nix_builtin(name: &str) -> bool {
    name.starts_with("builtins.")
        || name.starts_with("lib.")
        || name.starts_with("pkgs.")
        || name.starts_with("config.")
}

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
        if is_nix_builtin(target) {
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
}

pub static NIX_HOOKS: NixHooks = NixHooks;
