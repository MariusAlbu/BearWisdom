// =============================================================================
// matlab/resolve.rs — MATLAB resolution rules
//
// MATLAB module system:
//   - No explicit import statements in general code.
//   - `addpath('dir')` adds a directory to the search path at runtime.
//   - Each .m file defines one primary function or a classdef.
//   - Package directories are prefixed with `+`: `+mypackage/MyClass.m`.
//   - Within a classdef, methods reference other methods by name directly.
//
// Resolution strategy:
//   1. Scope chain walk (class method → class → file).
//   2. Same-file symbols (nested functions, local helpers).
//   3. Project-wide name lookup (each .m file is a callable unit).
// =============================================================================

use super::predicates;
use crate::indexer::resolve::engine::{
    self, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// MATLAB language resolver.
pub struct MatlabResolver;

impl LanguageResolver for MatlabResolver {
    fn language_ids(&self) -> &[&str] {
        &["matlab"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        let mut imports = Vec::new();

        // MATLAB uses addpath() rather than import directives, but the extractor
        // may emit EdgeKind::Imports for `import pkg.*` (OOP MATLAB). Collect
        // those here so downstream steps can use them.
        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            let module_path = r.module.clone().or_else(|| Some(r.target_name.clone()));
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path,
                alias: None,
                is_wildcard: r.target_name.ends_with(".*"),
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "matlab".to_string(),
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

        engine::resolve_common("matlab", file_ctx, ref_ctx, lookup, predicates::kind_compatible)
    }

    fn infer_external_namespace(
        &self,
        _file_ctx: &FileContext,
        _ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        None
    }

    fn infer_external_namespace_with_lookup(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
        lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        if let Some(ns) = self.infer_external_namespace(file_ctx, ref_ctx, project_ctx) {
            return Some(ns);
        }

        // MATLAB toolbox calls are bare names with no import declaration.
        // When the matlab_runtime walker has indexed the installed toolbox,
        // the target name appears in the external symbol table under an
        // `ext:matlab:...` path. Confirm the name is known external before
        // classifying — avoids false attribution for names the walker didn't
        // find (no toolbox installed, or name is truly unresolved).
        let target = &ref_ctx.extracted_ref.target_name;
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            return None;
        }
        let bare = target.split('.').next().unwrap_or(target);
        let hits = lookup.by_name(bare);
        if hits
            .iter()
            .any(|sym| sym.file_path.starts_with("ext:matlab:"))
        {
            return Some("matlab-runtime".to_string());
        }

        None
    }
}
