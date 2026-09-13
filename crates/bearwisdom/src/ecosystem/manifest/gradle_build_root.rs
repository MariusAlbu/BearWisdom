// ecosystem/manifest/gradle_build_root.rs — the Gradle build a package
// directory belongs to.
//
// Gradle resolves version catalogs (`gradle/*.versions.toml`), included
// builds (`includeBuild`) and `buildSrc` from the directory holding the
// build's `settings.gradle[.kts]`, not from the subproject that references
// them. A subproject's declarations are therefore read against its build
// root.

use std::path::{Path, PathBuf};

const SETTINGS_FILES: &[&str] = &["settings.gradle.kts", "settings.gradle"];

/// The nearest directory at or above `package_dir` — never above
/// `workspace_root` — that holds a Gradle settings file. `package_dir`
/// itself when no ancestor inside the workspace declares one.
pub(crate) fn gradle_build_root(workspace_root: &Path, package_dir: &Path) -> PathBuf {
    let mut dir = package_dir;
    loop {
        if has_settings_file(dir) {
            return dir.to_path_buf();
        }
        if dir == workspace_root {
            break;
        }
        match dir.parent() {
            Some(parent) if package_dir.starts_with(workspace_root) => dir = parent,
            _ => break,
        }
    }
    package_dir.to_path_buf()
}

fn has_settings_file(dir: &Path) -> bool {
    SETTINGS_FILES.iter().any(|name| dir.join(name).is_file())
}

#[cfg(test)]
#[path = "gradle_build_root_tests.rs"]
mod tests;
