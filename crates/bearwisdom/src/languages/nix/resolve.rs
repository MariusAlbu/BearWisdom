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

use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolLookup,
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
        if is_nix_builtin(target) {
            return None;
        }

        // Step 1: Same-file resolution.
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name == *target {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "nix_same_file",
                });
            }
        }

        // Step 2: Dotted attribute path — try as qualified name directly.
        if target.contains('.') {
            if let Some(sym) = lookup.by_qualified_name(target) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "nix_qualified_name",
                });
            }
            // Fall back to using just the last segment.
            let last_seg = target.rsplit('.').next().unwrap_or(target.as_str());
            let candidates = lookup.by_name(last_seg);
            if let Some(sym) = candidates.into_iter().next() {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.75,
                    strategy: "nix_attr_path_last_seg",
                });
            }
        }

        // Step 3: Global name lookup.
        let candidates = lookup.by_name(target);
        if let Some(sym) = candidates.into_iter().next() {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 0.85,
                strategy: "nix_global_name",
            });
        }

        None
    }

    fn infer_external_namespace(
        &self,
        _file_ctx: &FileContext,
        ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        // Import refs: classify by path type.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let path = ref_ctx
                .extracted_ref
                .module
                .as_deref()
                .unwrap_or(target.as_str());

            if path.starts_with('<') && path.ends_with('>') {
                // Channel reference like <nixpkgs> — always external.
                return Some(path.to_string());
            }
            // Relative paths are local files — not external.
            return None;
        }

        if is_nix_builtin(target) {
            return Some("builtin".to_string());
        }

        None
    }
}

/// Nix built-in functions and common stdlib attribute paths.
fn is_nix_builtin(name: &str) -> bool {
    // Direct builtins.* calls
    if name.starts_with("builtins.") || name.starts_with("lib.") || name.starts_with("pkgs.lib.") {
        return true;
    }
    matches!(
        name,
        // Nix language built-ins
        "import" | "builtins" | "derivation" | "abort" | "throw"
            | "toString" | "toJSON" | "fromJSON" | "toPath" | "isNull"
            | "isAttrs" | "isList" | "isString" | "isInt" | "isFloat"
            | "isBool" | "isFunction" | "isPath"
            | "map" | "filter" | "foldl'" | "foldl" | "foldr" | "head" | "tail"
            | "length" | "elem" | "elemAt" | "concatLists" | "concatMap"
            | "listToAttrs" | "attrNames" | "attrValues" | "hasAttr"
            | "getAttr" | "removeAttrs" | "mapAttrs" | "intersectAttrs"
            | "functionArgs" | "readFile" | "readDir" | "pathExists"
            | "fetchurl" | "fetchTarball" | "fetchGit" | "fetchFromGitHub"
            | "nixPath" | "storeDir" | "nixVersion"
            // Nixpkgs helpers
            | "callPackage" | "mkDerivation" | "stdenv" | "pkgs" | "lib"
            | "self" | "super" | "prev" | "final"
    )
}
