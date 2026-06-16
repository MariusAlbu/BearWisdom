// EJS language hooks.
//
// The plugin keeps a hook only to build the per-file resolution context (the
// `Imports` ref list with empty module paths). Partial-include resolution and
// bare-name refs are generic engine code: the former driven by the profile's
// `import_resolution` data, the latter by the generic strategy ladder.

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::legacy::{FileContext, ImportEntry};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct EjsHooks;

impl LanguageEngineHooks for EjsHooks {
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
                module_path: None,
                alias: None,
                is_wildcard: false,
            })
            .collect();
        Some(FileContext {
            file_path: file.path.clone(),
            language: "ejs".to_string(),
            imports,
            file_namespace: None,
        })
    }
}

pub static EJS_HOOKS: EjsHooks = EjsHooks;
