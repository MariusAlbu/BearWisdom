use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::EdgeKind;

pub struct DockerfileHooks;

impl LanguageEngineHooks for DockerfileHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        _file_ctx: &FileContext,
        _project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        // `scratch` is a special Docker pseudo-image.
        if ref_ctx.extracted_ref.target_name.eq_ignore_ascii_case("scratch") {
            return Some("docker".to_string());
        }

        // `FROM <image>` emits both Imports and Inherits edges targeting the
        // same base image. By the time we get here, multi-stage aliases have
        // already been resolved internally, so what's left is a registry
        // image reference.
        if matches!(
            ref_ctx.extracted_ref.kind,
            EdgeKind::Imports | EdgeKind::Inherits
        ) {
            return Some("docker".to_string());
        }

        None
    }
}

pub static DOCKERFILE_HOOKS: DockerfileHooks = DockerfileHooks;
