// =============================================================================
// nunjucks/hooks.rs — Nunjucks engine hooks.
//
// Nunjucks (`.njk`) is a Jinja2-compatible template DSL.
//   * `{% extends "base.njk" %}` and `{% include "partial.njk" %}` extract as
//     Imports refs whose `target_name` carries the template path. Their
//     resolution is generic engine code driven by the profile's
//     `import_resolution` data; the import's `module_path` echoes the target.
//   * `{{ expr }}` interpolation dispatches to JavaScript via embedded
//     regions; those refs carry `ref_origin_lang = "javascript"` and resolve
//     through the JS hook, not this one.
//
// The plugin keeps a hook only to build the per-file resolution context.
// =============================================================================

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, ImportEntry};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct NunjucksHooks;

impl LanguageEngineHooks for NunjucksHooks {
    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        let imports: Vec<ImportEntry> = file
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Imports)
            .map(|r| ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(r.target_name.clone()),
                alias: None,
                is_wildcard: false,
            })
            .collect();
        Some(FileContext {
            file_path: file.path.clone(),
            language: "nunjucks".to_string(),
            imports,
            file_namespace: None,
        })
    }
}

pub static NUNJUCKS_HOOKS: NunjucksHooks = NunjucksHooks;
