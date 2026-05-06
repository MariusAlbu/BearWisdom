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
        // MATLAB built-ins / toolbox functions are classified by the
        // engine's keywords() set populated from matlab/keywords.rs;
        // matlab_runtime walker emits real symbols for installed toolboxes.
        None
    }
}
