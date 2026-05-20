use super::resolve;
use super::predicates;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolLookup, Resolution};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::EdgeKind;

pub struct ScssHooks;

impl LanguageEngineHooks for ScssHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        _project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        // Property-value `call_expression` tagged by the extractor — CSS/SCSS
        // built-in function evaluation. Always external.
        if let Some(module) = &ref_ctx.extracted_ref.module {
            if module == super::extract::SCSS_CSS_FN_HINT {
                return Some("css".to_string());
            }
        }

        // @use / @forward of a Sass built-in module — path is the namespace
        // (e.g. "sass:math", "sass:color").
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let path = ref_ctx
                .extracted_ref
                .module
                .as_deref()
                .unwrap_or(target.as_str());
            if predicates::is_sass_builtin_module(path) {
                return Some(path.to_string());
            }
        }

        // `@use 'npm-package/path' as alias` — match the alias/imported_name
        // against this file's imports, classify the non-relative module path
        // as an external npm package.
        for import in &file_ctx.imports {
            let alias_matches = import.alias.as_deref() == Some(target.as_str())
                || import.imported_name == *target;
            if !alias_matches {
                continue;
            }
            if let Some(mp) = &import.module_path {
                if !mp.starts_with('.') && !mp.starts_with('/') {
                    let pkg = mp.split('/').next().unwrap_or(mp.as_str());
                    return Some(pkg.to_string());
                }
            }
        }

        None
    }

    fn build_file_context(
        &self,
        file: &crate::types::ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<crate::indexer::resolve::engine::FileContext> {
        Some(resolve::build_file_context_inner(file, project_ctx))
    }

    fn resolve_ref(
        &self,
        file_ctx: &crate::indexer::resolve::engine::FileContext,
        ref_ctx: &crate::indexer::resolve::engine::RefContext<'_>,
        lookup: &dyn crate::indexer::resolve::engine::SymbolLookup,
    ) -> Option<crate::indexer::resolve::engine::Resolution> {
        super::resolve::ScssResolver.resolve(file_ctx, ref_ctx, lookup)
    }
}

pub static SCSS_HOOKS: ScssHooks = ScssHooks;
