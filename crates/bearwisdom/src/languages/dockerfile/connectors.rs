// =============================================================================
// languages/dockerfile/connectors.rs — Dockerfile language plugin connectors
//
// Contains the Docker Compose post-index hook.  Docker Compose files are not
// Dockerfile language files, but they belong conceptually to the same
// infrastructure layer, so this plugin owns the hook.
// =============================================================================

use std::path::Path;

use tracing::warn;

/// Detect Docker Compose service dependencies and write flow_edges.
///
/// Called from `DockerfilePlugin::post_index()`.
pub fn run_docker_compose(db: &crate::db::Database, project_root: &Path) {
    match crate::connectors::docker_compose::connect(db, project_root) {
        Ok(n) if n > 0 => tracing::info!(n, "Docker Compose service dependency edges"),
        Err(e) => warn!("Docker Compose connector: {e}"),
        _ => {}
    }
}
