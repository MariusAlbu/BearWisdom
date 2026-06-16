// Jinja2 language hooks.

use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::legacy::{FileContext, ImportEntry, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct JinjaHooks;

pub(crate) fn infer_ansible_external(
    target: &str,
    project_ctx: Option<&ProjectContext>,
) -> Option<String> {
    let ctx = project_ctx?;
    let manifest = ctx.manifests.get(&ManifestKind::AnsibleRequirements)?;
    for role in &manifest.dependencies {
        let prefix = format!("{role}_");
        if target.starts_with(prefix.as_str()) || target == role.as_str() {
            return Some(format!("ansible.{role}"));
        }
    }
    None
}

impl LanguageEngineHooks for JinjaHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        _file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        infer_ansible_external(ref_ctx.extracted_ref.target_name.as_str(), project_ctx)
    }

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
            language: "jinja".to_string(),
            imports,
            file_namespace: None,
        })
    }
}

pub static JINJA_HOOKS: JinjaHooks = JinjaHooks;
