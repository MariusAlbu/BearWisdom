// Jinja hooks — classify_external delegates to the resolver's
// Ansible-role classifier.

use super::resolve::infer_ansible_external;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolLookup};
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
}

pub static JINJA_HOOKS: JinjaHooks = JinjaHooks;
