// =============================================================================
// languages/hcl/connectors.rs — HCL language plugin connectors
//
// Contains the Kubernetes manifest post-index hook.  Kubernetes YAML is not
// HCL/Terraform, but both describe infrastructure, so this plugin owns the hook.
// =============================================================================

use std::path::Path;

use tracing::warn;

/// Detect Kubernetes service references and write flow_edges.
///
/// Called from `HclPlugin::post_index()`.
pub fn run_kubernetes(db: &crate::db::Database, project_root: &Path) {
    match crate::connectors::kubernetes::connect(db, project_root) {
        Ok(n) if n > 0 => tracing::info!(n, "Kubernetes service reference edges"),
        Err(e) => warn!("Kubernetes connector: {e}"),
        _ => {}
    }
}
