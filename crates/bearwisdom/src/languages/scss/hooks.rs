// SCSS language hooks. Absorbed from the deleted `scss/resolve.rs`.

use super::predicates;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, ImportEntry, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct ScssHooks;

/// A ref's `module` names a non-project provider the binder must never resolve
/// through: a Sass built-in module (`@use 'sass:math'`) or the synthesized
/// CSS-function-call hint. Drives the profile's `module_skip` so a module-
/// carrying ref declines before the ladder and external classification brands
/// it. The target-keyed `builtin_skip` sibling, keyed on `r.module`.
pub(crate) fn is_scss_skippable_module(module: &str) -> bool {
    module == super::extract::SCSS_CSS_FN_HINT || predicates::is_sass_builtin_module(module)
}

impl LanguageEngineHooks for ScssHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        _project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;
        if let Some(module) = &ref_ctx.extracted_ref.module {
            if module == super::extract::SCSS_CSS_FN_HINT {
                return Some("css".to_string());
            }
        }
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
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        let mut imports = Vec::new();
        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            let module_path = r.module.clone().unwrap_or_else(|| r.target_name.clone());
            let bare_segment = module_path
                .rsplit('/')
                .next()
                .unwrap_or(module_path.as_str())
                .trim_start_matches('_')
                .trim_end_matches(".scss")
                .trim_end_matches(".sass")
                .trim_end_matches(".css");
            let alias = if r.target_name != bare_segment {
                Some(r.target_name.clone())
            } else {
                None
            };
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(module_path),
                alias,
                is_wildcard: false,
            });
        }
        Some(FileContext {
            file_path: file.path.clone(),
            language: "scss".to_string(),
            imports,
            file_namespace: None,
        })
    }
}

pub static SCSS_HOOKS: ScssHooks = ScssHooks;
