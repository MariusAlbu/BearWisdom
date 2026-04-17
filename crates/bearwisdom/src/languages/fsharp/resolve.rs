// =============================================================================
// fsharp/resolve.rs — F# resolution rules
//
// Scope rules for F#:
//
//   1. Scope chain walk: innermost let-binding / function → module → namespace.
//   2. Same-file resolution: all top-level bindings in the file are visible.
//   3. Import-based resolution: `open Namespace.Module` and
//      `open type TypeName` bring symbols into scope.
//
// F# import model:
//   `open Namespace.Module`   → wildcard open; all public members in scope
//   `open type TypeName`      → static members of a type in scope
// =============================================================================

use super::predicates;
use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution,
    SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// F# language resolver.
pub struct FSharpResolver;

impl LanguageResolver for FSharpResolver {
    fn language_ids(&self) -> &[&str] {
        &["fsharp"]
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
            // target_name is the opened namespace/module path.
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(r.target_name.clone()),
                alias: None,
                is_wildcard: true,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "fsharp".to_string(),
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

        if edge_kind == EdgeKind::Imports {
            return None;
        }

        // F# stdlib and language keywords are not in the project index.
        if predicates::is_fsharp_builtin(target) {
            return None;
        }

        engine::resolve_common("fsharp", file_ctx, ref_ctx, lookup, predicates::kind_compatible)
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        // Import refs (`open System.Linq`) — classify via NuGet manifest.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let external = match project_ctx {
                Some(ctx) => is_manifest_external_namespace(ctx, target),
                None => predicates::is_external_namespace_fallback(target),
            };
            if external {
                let root = target.split('.').next().unwrap_or(target);
                return Some(root.to_string());
            }
            return None;
        }

        // F# core builtins (printfn, Seq.map, etc.).
        if predicates::is_fsharp_builtin(target) {
            return Some("FSharp.Core".to_string());
        }

        // Module-qualified ref: if the ref has module="X" and X is an
        // external namespace (from NuGet packages or SDK), classify it.
        if let Some(module) = &ref_ctx.extracted_ref.module {
            let external = match project_ctx {
                Some(ctx) => is_manifest_external_namespace(ctx, module),
                None => predicates::is_external_namespace_fallback(module),
            };
            if external {
                let root = module.split('.').next().unwrap_or(module);
                return Some(root.to_string());
            }
        }

        // Check file's open declarations: if the target was brought in via
        // `open ExternalNamespace`, classify it.
        for import in &file_ctx.imports {
            let Some(module_path) = &import.module_path else { continue };
            let external = match project_ctx {
                Some(ctx) => is_manifest_external_namespace(ctx, module_path),
                None => predicates::is_external_namespace_fallback(module_path),
            };
            if external {
                let root = module_path.split('.').next().unwrap_or(module_path);
                return Some(root.to_string());
            }
        }

        None
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Check whether a .NET namespace is external, using the NuGet manifest directly.
///
/// Mirrors the C# resolver's logic: System/Microsoft are always external; NuGet
/// package names are checked as namespace prefixes and root-segment matches.
fn is_manifest_external_namespace(ctx: &ProjectContext, ns: &str) -> bool {
    let root = ns.split('.').next().unwrap_or(ns);
    if matches!(root, "System" | "Microsoft") {
        return true;
    }
    if let Some(m) = ctx.manifest(ManifestKind::NuGet) {
        if m.dependencies.contains(ns) {
            return true;
        }
        for dep in &m.dependencies {
            if ns.starts_with(dep.as_str())
                && ns.len() > dep.len()
                && ns.as_bytes()[dep.len()] == b'.'
            {
                return true;
            }
            if let Some(dep_root) = dep.split('.').next() {
                if root == dep_root {
                    return true;
                }
            }
        }
        return false;
    }
    false
}
