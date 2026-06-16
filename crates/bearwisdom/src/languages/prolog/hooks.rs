// Prolog language hooks. Absorbed from the deleted `prolog/resolve.rs`.

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::legacy::{FileContext, ImportEntry};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct PrologHooks;

impl LanguageEngineHooks for PrologHooks {
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
            let module_path = r.module.clone().or_else(|| Some(r.target_name.clone()));
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path,
                alias: None,
                is_wildcard: true,
            });
        }
        Some(FileContext {
            file_path: file.path.clone(),
            language: "prolog".to_string(),
            imports,
            file_namespace: None,
        })
    }
}

pub static PROLOG_HOOKS: PrologHooks = PrologHooks;
