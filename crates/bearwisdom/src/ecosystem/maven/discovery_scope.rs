// =============================================================================
// ecosystem/maven/discovery_scope.rs — where JVM dependency discovery reads
// its declarations from
// =============================================================================

use std::path::{Path, PathBuf};

use crate::ecosystem::manifest::gradle_build_root::gradle_build_root;

/// The two directories JVM discovery reads: the package whose own manifests
/// and imports drive the demand, and the Gradle build root that owns the
/// version catalogs, build logic and settings-declared modules the package
/// shares with its siblings. Both are the same directory for a
/// single-project layout.
pub(crate) struct JvmDiscoveryScope {
    pub build_root: PathBuf,
    pub package_dir: PathBuf,
}

impl JvmDiscoveryScope {
    /// One build rooted at `project_root`.
    pub(crate) fn whole(project_root: &Path) -> Self {
        Self {
            build_root: project_root.to_path_buf(),
            package_dir: project_root.to_path_buf(),
        }
    }

    /// One workspace package, read against the Gradle build it belongs to.
    pub(crate) fn for_package(workspace_root: &Path, package_dir: &Path) -> Self {
        Self {
            build_root: gradle_build_root(workspace_root, package_dir),
            package_dir: package_dir.to_path_buf(),
        }
    }
}
