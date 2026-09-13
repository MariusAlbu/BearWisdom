// =============================================================================
// ecosystem/maven/locator.rs — the JVM ecosystem as an `ExternalSourceLocator`
// =============================================================================

use std::path::Path;
use std::sync::Arc;

use super::discovery::discover_maven_roots;
use super::discovery_scope::JvmDiscoveryScope;
use super::{walk_maven_root, MavenEcosystem, ID};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::walker::WalkedFile;

impl ExternalSourceLocator for MavenEcosystem {
    fn ecosystem(&self) -> &'static str {
        ID.as_str()
    }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_maven_roots(&JvmDiscoveryScope::whole(project_root))
    }

    /// A workspace package's declarations are read against the Gradle build
    /// it belongs to, so catalog accessors and convention-plugin
    /// declarations at the settings root reach every subproject.
    fn locate_roots_for_package(
        &self,
        workspace_root: &Path,
        package_abs_path: &Path,
        package_id: i64,
    ) -> Vec<ExternalDepRoot> {
        let scope = JvmDiscoveryScope::for_package(workspace_root, package_abs_path);
        let mut roots = discover_maven_roots(&scope);
        for root in &mut roots {
            root.package_id = Some(package_id);
        }
        roots
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_maven_root(dep)
    }
}

/// Process-wide shared instance. The ecosystem registry holds one of these
/// in `default_registry()`; the legacy-locator bridge in
/// `ecosystem::default_locator` exposes the same type through
/// `ExternalSourceLocator` for per-package attribution overrides.
pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<MavenEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(MavenEcosystem)).clone()
}
