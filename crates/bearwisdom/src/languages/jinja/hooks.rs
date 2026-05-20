// Jinja hooks — classify_external delegates to the resolver's
// Ansible-role classifier.

use super::resolve;
use super::resolve::infer_ansible_external;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolLookup, Resolution};
use crate::type_checker::profile::hooks::LanguageEngineHooks;

pub struct JinjaHooks;

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
        super::resolve::JinjaResolver.resolve(file_ctx, ref_ctx, lookup)
    }
}

pub static JINJA_HOOKS: JinjaHooks = JinjaHooks;
