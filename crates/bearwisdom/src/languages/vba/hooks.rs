// VBA language hooks. Absorbed from the deleted `vba/resolve.rs`.

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{self as engine, FileContext, ImportEntry};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct VbaHooks;

impl LanguageEngineHooks for VbaHooks {
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
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: r.module.clone(),
                alias: None,
                is_wildcard: false,
            });
        }
        Some(FileContext {
            file_path: file.path.clone(),
            language: "vba".to_string(),
            imports,
            file_namespace: None,
        })
    }
}

pub static VBA_HOOKS: VbaHooks = VbaHooks;
