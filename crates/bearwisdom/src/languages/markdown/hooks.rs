use super::resolve;
use crate::indexer::project_context::ProjectContext;
use crate::type_checker::profile::hooks::LanguageEngineHooks;

pub struct MarkdownHooks;

impl LanguageEngineHooks for MarkdownHooks {
    fn build_file_context(
        &self,
        file: &crate::types::ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<crate::indexer::resolve::engine::FileContext> {
        Some(resolve::build_file_context_inner(file, project_ctx))
    }
}

pub static MARKDOWN_HOOKS: MarkdownHooks = MarkdownHooks;