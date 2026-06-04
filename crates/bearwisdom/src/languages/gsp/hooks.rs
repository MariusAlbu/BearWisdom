// GSP language hooks.
//
// The plugin keeps a hook only to build the per-file resolution context. GSP
// carries no symbol-level imports — `<g:render template="...">` resolution
// reads the ref's target name directly — so the import list is empty. Template
// binding itself is generic engine code driven by the profile's
// `import_resolution` data.

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::FileContext;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::ParsedFile;

pub struct GspHooks;

impl LanguageEngineHooks for GspHooks {
    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        Some(FileContext {
            file_path: file.path.clone(),
            language: "gsp".to_string(),
            imports: Vec::new(),
            file_namespace: None,
        })
    }
}

pub static GSP_HOOKS: GspHooks = GspHooks;

#[cfg(test)]
#[path = "hooks_tests.rs"]
mod hooks_tests;
