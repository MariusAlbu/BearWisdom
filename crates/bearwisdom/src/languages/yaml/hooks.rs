// YAML language hooks.
//
// The plugin keeps a hook only to build the per-file resolution context (the
// `Imports` ref list with empty module paths). GitHub-Actions `uses:`
// resolution is generic engine code driven by the profile's
// `import_resolution` data.

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, ImportEntry};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct YamlHooks;

impl LanguageEngineHooks for YamlHooks {
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
            language: "yaml".to_string(),
            imports,
            file_namespace: None,
        })
    }
}

pub static YAML_HOOKS: YamlHooks = YamlHooks;
