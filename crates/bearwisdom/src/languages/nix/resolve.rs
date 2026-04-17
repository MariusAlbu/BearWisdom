// =============================================================================
// languages/nix/resolve.rs — Nix expression language resolution rules
//
// Nix reference forms emitted by the extractor:
//
//   import ./path/to/file.nix  → Imports, target_name = "./path/...", module = same
//   import <nixpkgs>           → Imports, target_name = "<nixpkgs>",  module = same
//   callPackage ./pkg {}       → Imports, target_name = "./pkg",       module = same
//   functionName arg           → Calls,   target_name = "functionName", module = None
//   pkgs.lib.attrsets.foo      → Calls,   target_name = "pkgs.lib.attrsets.foo"
//   with_expression target     → Imports, target_name = varname, module = None
//
// Resolution strategy:
//   1. Imports (path-based): mark as external/local — Nix imports are file paths,
//      not indexed symbols. `infer_external_namespace` handles classification.
//   2. Calls — same-file lookup first, then global by_name lookup.
//   3. Dotted names (attribute paths): try as a qualified_name, then take the
//      last segment as a bare name.
//   4. Nix built-ins (builtins.*, lib.*): mark external.
// =============================================================================

use super::predicates;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution,
    SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

pub struct NixResolver;

impl LanguageResolver for NixResolver {
    fn language_ids(&self) -> &[&str] {
        &["nix"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
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

        FileContext {
            file_path: file.path.clone(),
            language: "nix".to_string(),
            imports,
            file_namespace: None,
        }
    }

    fn resolve(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        let target = &ref_ctx.extracted_ref.target_name;
        let edge_kind = ref_ctx.extracted_ref.kind;

        // Import refs are path declarations, not symbol references.
        if edge_kind == EdgeKind::Imports {
            return None;
        }

        // Skip Nix built-in attribute paths.
        if predicates::is_nix_builtin(target) {
            return None;
        }

        // Language-specific: dotted attribute path — try as qualified name directly,
        // then fall back to the last segment. This is Nix-specific and happens before
        // the shared resolution steps.
        if target.contains('.') {
            if let Some(sym) = lookup.by_qualified_name(target.as_str()) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "nix_qualified_name",
                });
            }
            let last_seg = target.rsplit('.').next().unwrap_or(target.as_str());
            if let Some(sym) = lookup.by_name(last_seg).first() {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.75,
                    strategy: "nix_attr_path_last_seg",
                });
            }
        }

        engine::resolve_common("nix", file_ctx, ref_ctx, lookup, |_, _| true)
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        // Language-specific: Nix channel refs like <nixpkgs> are external; relative
        // path imports are local and must NOT be marked external.
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

        engine::infer_external_common(file_ctx, ref_ctx, project_ctx, predicates::is_nix_builtin)
    }
}
