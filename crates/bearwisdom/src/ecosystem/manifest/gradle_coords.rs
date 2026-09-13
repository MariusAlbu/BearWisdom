// ecosystem/manifest/gradle_coords.rs — declared Maven coordinates of a
// Gradle project.
//
// Parses every Gradle DSL file that can carry a dependency declaration — the
// ordinary `build.gradle[.kts]` files plus the build-logic sources — and
// resolves `<catalog>.<accessor>` references against the project's
// `gradle/*.versions.toml` catalogs.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::gradle;
use super::gradle_build_logic;
use super::maven::MavenCoord;

/// Full `MavenCoord`s declared by a Gradle project. Coords that omit
/// `version` (rare in Gradle) are returned version-less; the caller applies
/// its version-dir scan fallback the same way it does for pom coords.
pub(crate) fn collect_gradle_coords(project_root: &Path) -> Vec<MavenCoord> {
    collect_gradle_coords_scoped(project_root, project_root)
}

/// Coordinates declared by the build files under `package_dir`, plus those
/// declared by the build logic of the Gradle build at `build_root`, with
/// catalog accessors resolved against that build's version catalogs.
/// Convention plugins carry no record of which subproject applies them, so
/// their declarations count for every package of the build.
pub(crate) fn collect_gradle_coords_scoped(
    build_root: &Path,
    package_dir: &Path,
) -> Vec<MavenCoord> {
    let catalogs = read_version_catalogs(build_root);

    let mut files: Vec<PathBuf> = gradle::collect_gradle_build_files(package_dir);
    files.extend(gradle_build_logic::collect_gradle_build_logic_files(
        build_root,
    ));

    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut out = Vec::new();
    for file in files {
        if !seen.insert(file.clone()) {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&file) else {
            continue;
        };
        out.extend(gradle::parse_gradle_coords(&content, &catalogs));
    }
    out
}

/// Every `gradle/*.versions.toml` catalog in the project, keyed by the
/// accessor name the DSL uses to reference it.
fn read_version_catalogs(project_root: &Path) -> HashMap<String, gradle::GradleCatalog> {
    let mut catalogs = HashMap::new();
    for (name, path) in gradle::collect_version_catalogs(project_root) {
        if let Ok(content) = std::fs::read_to_string(&path) {
            catalogs.insert(name, gradle::parse_version_catalog(&content));
        }
    }
    catalogs
}

#[cfg(test)]
#[path = "gradle_coords_tests.rs"]
mod tests;
