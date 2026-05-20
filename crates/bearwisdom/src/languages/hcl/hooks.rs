use super::resolve::{
    is_dynamic_block_iterator, is_provider_resource_type, is_terraform_meta_ref,
};
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::EdgeKind;

pub struct HclHooks;

impl LanguageEngineHooks for HclHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        _file_ctx: &FileContext,
        _project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            return Some("terraform".to_string());
        }
        if is_terraform_meta_ref(target) {
            return Some("terraform".to_string());
        }
        if is_dynamic_block_iterator(target) {
            return Some("terraform".to_string());
        }
        if target.starts_with("data.") {
            let parts: Vec<&str> = target.splitn(3, '.').collect();
            if parts.len() >= 2 && is_provider_resource_type(parts[1]) {
                return Some("terraform".to_string());
            }
        }
        if let Some(dot) = target.find('.') {
            let prefix = &target[..dot];
            if is_provider_resource_type(prefix) {
                return Some("terraform".to_string());
            }
        }
        None
    }
}

pub static HCL_HOOKS: HclHooks = HclHooks;
