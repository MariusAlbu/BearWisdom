// =============================================================================
// templ/hooks.rs — Templ engine hooks.
//
// Templ (`.templ`) is an HTML-component DSL compiled to Go. A `templ Foo(args)`
// declaration extracts as a Function; `@Bar(args)` inside a templ body extracts
// as a Calls ref to `Bar`. The engine's generic resolver tower handles same-file
// lookups (most templ components live in the same package directory), gated by
// the profile's `kind_compatible_table`.
// =============================================================================

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, ImportEntry};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct TemplHooks;

impl LanguageEngineHooks for TemplHooks {
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
                module_path: r.module.clone(),
                alias: None,
                is_wildcard: false,
            })
            .collect();
        Some(FileContext {
            file_path: file.path.clone(),
            language: "templ".to_string(),
            imports,
            file_namespace: None,
        })
    }
}

pub static TEMPL_HOOKS: TemplHooks = TemplHooks;
