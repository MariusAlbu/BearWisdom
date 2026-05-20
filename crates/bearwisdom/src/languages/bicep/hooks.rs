use super::resolve::{is_azure_resource_type, is_child_resource_shorthand};
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{self as engine, FileContext, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::EdgeKind;

pub struct BicepHooks;

impl LanguageEngineHooks for BicepHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;
        let edge_kind = ref_ctx.extracted_ref.kind;

        // Azure resource type strings take priority.
        if is_azure_resource_type(target) {
            return Some("azure".to_string());
        }

        // Child-resource shorthand: nested `resource childAlias 'subnets'`
        // uses a bare single-segment type that resolves against the parent's
        // type path at deploy time. Bicep emits TypeRef refs only for
        // resource-declaration type strings, so any bare-name TypeRef here
        // is a child shorthand.
        if edge_kind == EdgeKind::TypeRef && is_child_resource_shorthand(target) {
            return Some("azure".to_string());
        }

        // No predicate-based builtin classification — builtin names come
        // from the `bicep-runtime` ecosystem walker via the symbol index.
        engine::infer_external_common(file_ctx, ref_ctx, project_ctx, |_| false)
    }
}

pub static BICEP_HOOKS: BicepHooks = BicepHooks;
