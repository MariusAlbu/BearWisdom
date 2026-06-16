// Dockerfile language hooks.

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::legacy::{FileContext, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

/// A `FROM` instruction's base-image module carries a registry-image shape — a
/// `:` tag (`ubuntu:latest`) or a `/` registry/repository path
/// (`docker.io/library/node`). Such a target names a published image, never a
/// project build-stage, so it declines before the ladder rather than binding a
/// same-named local homonym; external classification brands it after. A bare
/// untagged image (`FROM node`) carries neither shape and stays on the ladder —
/// it is structurally indistinguishable from a multi-stage `FROM <stage>` reuse.
pub(crate) fn is_registry_image_module(module: &str) -> bool {
    module.contains(':') || module.contains('/')
}

pub struct DockerfileHooks;

impl LanguageEngineHooks for DockerfileHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        _file_ctx: &FileContext,
        _project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        if ref_ctx
            .extracted_ref
            .target_name
            .eq_ignore_ascii_case("scratch")
        {
            return Some("docker".to_string());
        }
        if matches!(
            ref_ctx.extracted_ref.kind,
            EdgeKind::Imports | EdgeKind::Inherits
        ) {
            return Some("docker".to_string());
        }
        None
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        Some(FileContext {
            file_path: file.path.clone(),
            language: "dockerfile".to_string(),
            imports: Vec::new(),
            file_namespace: None,
        })
    }
}

pub static DOCKERFILE_HOOKS: DockerfileHooks = DockerfileHooks;
