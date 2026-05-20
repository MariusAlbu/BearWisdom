// Dockerfile language hooks. Absorbed contents of the deleted
// `dockerfile/resolve.rs` — the resolver struct now lives here as a
// language-private inherent impl.

use super::predicates;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, RefContext, Resolution, SymbolLookup,
};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct DockerfileHooks;

impl LanguageEngineHooks for DockerfileHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        _file_ctx: &FileContext,
        _project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        if ref_ctx.extracted_ref.target_name.eq_ignore_ascii_case("scratch") {
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

    fn resolve_ref(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            return None;
        }
        engine::resolve_common(
            "dockerfile",
            file_ctx,
            ref_ctx,
            lookup,
            predicates::kind_compatible,
        )
    }
}

pub static DOCKERFILE_HOOKS: DockerfileHooks = DockerfileHooks;
