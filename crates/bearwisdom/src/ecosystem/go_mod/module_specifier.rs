//! Go-module resolver inputs owned by the Go ecosystem.

use crate::indexer::project_context::ProjectContext;

/// The project-local module prefix used to distinguish an internal Go import
/// from an external dependency.  Manifest selection and spelling live here,
/// beside the Go ecosystem rather than in the generic resolver engine.
pub(crate) fn project_module_path(ctx: &ProjectContext) -> Option<String> {
    ctx.manifests
        .get(&crate::ecosystem::manifest::ManifestKind::GoMod)
        .and_then(|manifest| manifest.module_path.clone())
}

#[cfg(test)]
mod tests {
    use super::project_module_path;
    use crate::ecosystem::manifest::{ManifestData, ManifestKind};
    use crate::indexer::project_context::ProjectContext;

    #[test]
    fn reads_the_go_ecosystems_own_manifest_signal() {
        let mut context = ProjectContext::default();
        context.manifests.insert(
            ManifestKind::GoMod,
            ManifestData {
                module_path: Some("example.test/project".into()),
                ..Default::default()
            },
        );
        assert_eq!(
            project_module_path(&context).as_deref(),
            Some("example.test/project")
        );
    }
}
