// =============================================================================
// ecosystem/maven/walk.rs — the source files of one extracted JVM root
//
// One walk for every JVM language: the file extension tags each file's
// language, so a Scala sources jar holding both `.scala` and `.java` files
// tags each correctly. Test source sets and test-suffixed files are left
// out; a package directory that happens to be named `test` (`kotlin/test`)
// is source like any other.
// =============================================================================

use std::path::Path;

use super::detect_jvm_language;
use crate::ecosystem::externals::{ExternalDepRoot, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;

/// A `test`/`tests` directory is a test source set only in the build layout
/// that puts it beside `main` under `src` (`src/test/java`); anywhere else it
/// is a package segment.
fn is_test_source_set(parent: &Path, name: &str) -> bool {
    matches!(name, "test" | "tests") && parent.file_name().and_then(|n| n.to_str()) == Some("src")
}

pub(crate) fn walk_maven_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    walk_generic_jvm_root(dep)
}

/// Walk any JVM source tree — reused by the Android SDK ecosystem whose
/// extracted sources follow the exact same layout (Java + Kotlin + Scala +
/// Clojure intermixed under a single cache dir).
pub(crate) fn walk_generic_jvm_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_dir_bounded(&dep.root, &dep.root, dep, &mut out, 0);
    out
}

fn walk_dir_bounded(
    dir: &Path,
    root: &Path,
    dep: &ExternalDepRoot,
    out: &mut Vec<WalkedFile>,
    depth: u32,
) {
    if depth >= MAX_WALK_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name == "META-INF" || name.starts_with('.') || is_test_source_set(dir, name) {
                    continue;
                }
            }
            walk_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };

            let (language, virtual_tag) = match detect_jvm_language(name) {
                Some(spec) => spec,
                None => continue,
            };

            // Skip test-suffixed files by convention.
            if name.ends_with("Test.java")
                || name.ends_with("Tests.java")
                || name.ends_with("Test.scala")
                || name.ends_with("Tests.scala")
                || name.ends_with("Spec.scala")
                || name.ends_with("Suite.scala")
                || name == "package-info.java"
                || name == "module-info.java"
            {
                continue;
            }

            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            let virtual_path = format!("ext:{virtual_tag}:{}/{}", dep.module_path, rel_sub);

            out.push(WalkedFile {
                relative_path: virtual_path,
                absolute_path: path,
                language,
            });
        }
    }
}

#[cfg(test)]
#[path = "walk_tests.rs"]
mod tests;
