// =============================================================================
// r_lang/resolve.rs — R resolution rules
//
// Scope rules for R:
//
//   1. Scope chain walk: innermost function → outermost (lexical scoping).
//   2. Same-file resolution: all top-level assignments are visible within
//      the file.
//   3. By-name lookup: for loaded packages, symbols may be defined elsewhere.
//
// R import model:
//   `library(pkg)`       → target_name = "pkg", EdgeKind::Imports (wildcard)
//   `require(pkg)`       → target_name = "pkg", EdgeKind::Imports (wildcard)
//   `source("file.R")`   → target_name = "file.R", EdgeKind::Imports
//   `pkg::fn`            → target_name = "fn", module = "pkg", EdgeKind::Calls
//
// Notes on non-standard evaluation (NSE):
//   Calls like `mutate(df, new_col = old_col + 1)` operate in a data-frame
//   context where `new_col` and `old_col` are column names, not variable refs.
//   The extractor emits a single Calls edge for `mutate` itself; the arguments
//   are not separately resolved.
//
// Formula operators (`~`):
//   `y ~ x + z` uses `~` as a language primitive. The extractor emits a Calls
//   ref with target_name = "~", which is classified as builtin here.
// =============================================================================

use super::predicates;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution,
    SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// R language resolver.
pub struct RResolver;

impl LanguageResolver for RResolver {
    fn language_ids(&self) -> &[&str] {
        &["r"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        let mut imports = Vec::new();

        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            // `library(pkg)` / `require(pkg)` are wildcard imports — every
            // exported symbol from the package is brought into scope.
            // `source("file.R")` is also treated as wildcard (all top-level
            // symbols from the sourced file become visible).
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(r.target_name.clone()),
                alias: None,
                is_wildcard: true,
            });
        }

        // R packages declare their namespace imports via `DESCRIPTION` Imports /
        // Depends fields (parsed at index time into ManifestKind::Description).
        // These don't produce explicit `library()` calls in source files — they
        // are package-level implicit wildcard imports. Add a wildcard entry for
        // every declared dep so the resolver can classify bare function calls
        // (e.g. `abort()` from `rlang`) as external.
        //
        // We add these as *lower-priority* entries after any explicit library()
        // calls; if a library() call already added the package, the resolver
        // sees it once. Duplicates are harmless — the engine stops on first
        // matching wildcard that passes the manifest check.
        if let Some(ctx) = project_ctx {
            let all_deps = ctx.all_dependency_names();
            for dep in &all_deps {
                // Skip base R "packages" that are never external.
                if matches!(dep.as_str(), "methods" | "utils" | "stats" | "base"
                    | "datasets" | "grDevices" | "graphics" | "tools") {
                    continue;
                }
                // Only add if not already present from an explicit library() call.
                if !imports.iter().any(|i| i.module_path.as_deref() == Some(dep)) {
                    imports.push(ImportEntry {
                        imported_name: dep.clone(),
                        module_path: Some(dep.clone()),
                        alias: None,
                        is_wildcard: true,
                    });
                }
            }
        }

        FileContext {
            file_path: file.path.clone(),
            language: "r".to_string(),
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
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            return None;
        }

        let target = &ref_ctx.extracted_ref.target_name;

        // R builtins (including formula operator `~`) are never in the index.
        if predicates::is_r_builtin(target) {
            return None;
        }

        // Namespace-qualified call to a known external package (e.g. dplyr::filter).
        // Skip index lookup — the function is defined in the package, not the project.
        if let Some(module) = &ref_ctx.extracted_ref.module {
            if predicates::is_r_package(module) {
                return None;
            }
        }

        resolve_r("r", file_ctx, ref_ctx, lookup)
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        // Import refs: the package name IS the external namespace.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            return Some(target.clone());
        }

        // Namespace-qualified ref to a known R package (pkg::fn).
        if let Some(module) = &ref_ctx.extracted_ref.module {
            if predicates::is_r_package(module) {
                return Some(module.clone());
            }
        }

        infer_r_external(file_ctx, ref_ctx, project_ctx)
    }
}

// ---------------------------------------------------------------------------
// Private helpers — thin wrappers around the engine so we can add R-specific
// logic without duplicating the whole resolve_common call graph.
// ---------------------------------------------------------------------------

fn resolve_r(
    lang_prefix: &'static str,
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    engine::resolve_common(lang_prefix, file_ctx, ref_ctx, lookup, predicates::kind_compatible)
}

fn infer_r_external(
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    project_ctx: Option<&ProjectContext>,
) -> Option<String> {
    engine::infer_external_common(file_ctx, ref_ctx, project_ctx, predicates::is_r_builtin)
}
