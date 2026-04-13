// Swift / SPM externals

use super::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;
use std::path::Path;
use tracing::debug;

/// Swift SPM → `discover_swift_externals` + `walk_swift_external_root`.
///
/// SPM fetches deps into `.build/checkouts/<dep>/`.
/// Declared deps come from `Package.swift` `.package(url:...)` calls.
/// Walk: `Sources/**/*.swift`, skipping `Tests/`, `Examples/`, `Benchmarks/`.
pub struct SwiftExternalsLocator;

impl ExternalSourceLocator for SwiftExternalsLocator {
    fn ecosystem(&self) -> &'static str { "swift" }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_swift_externals(project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_swift_external_root(dep)
    }
}

pub fn discover_swift_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    use crate::indexer::manifest::swift_pm::parse_swift_package_deps;

    let package_swift = project_root.join("Package.swift");
    if !package_swift.is_file() { return Vec::new(); }
    let Ok(content) = std::fs::read_to_string(&package_swift) else { return Vec::new(); };
    let declared = parse_swift_package_deps(&content);
    if declared.is_empty() { return Vec::new(); }

    let checkouts = project_root.join(".build").join("checkouts");
    if !checkouts.is_dir() { return Vec::new(); }

    let mut roots = Vec::new();
    for dep in &declared {
        let dep_dir = checkouts.join(dep);
        if dep_dir.is_dir() {
            roots.push(ExternalDepRoot {
                module_path: dep.clone(),
                version: String::new(),
                root: dep_dir,
                ecosystem: "swift",
            });
        }
    }
    debug!("Swift: discovered {} external package roots", roots.len());
    roots
}

pub fn walk_swift_external_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    let sources = dep.root.join("Sources");
    let walk_root = if sources.is_dir() { sources } else { dep.root.clone() };
    walk_swift_dir(&walk_root, &dep.root, dep, &mut out);
    out
}

fn walk_swift_dir(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>) {
    walk_swift_dir_bounded(dir, root, dep, out, 0);
}

fn walk_swift_dir_bounded(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= MAX_WALK_DEPTH { return; }
    let Ok(entries) = std::fs::read_dir(dir) else { return; };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else { continue; };
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "Tests" | "tests" | "Examples" | "Benchmarks") || name.starts_with('.') { continue; }
            }
            walk_swift_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue; };
            if !name.ends_with(".swift") { continue; }
            if name.ends_with("Tests.swift") || name.ends_with("Test.swift") { continue; }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            out.push(WalkedFile {
                relative_path: format!("ext:swift:{}/{}", dep.module_path, rel_sub),
                absolute_path: path,
                language: "swift",
            });
        }
    }
}
