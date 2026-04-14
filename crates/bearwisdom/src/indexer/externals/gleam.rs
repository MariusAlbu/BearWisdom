// Gleam externals

use super::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;
use std::path::Path;
use tracing::debug;

/// Gleam → `discover_gleam_externals` + `walk_gleam_external_root`.
///
/// Gleam fetches deps into `build/packages/<dep>/`.
/// Declared deps come from `gleam.toml` `[dependencies]` section.
/// Walk: `src/**/*.gleam`, skipping `test/`.
pub struct GleamExternalsLocator;

impl ExternalSourceLocator for GleamExternalsLocator {
    fn ecosystem(&self) -> &'static str { "gleam" }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_gleam_externals(project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_gleam_external_root(dep)
    }
}

pub fn discover_gleam_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    use crate::indexer::manifest::gleam::parse_gleam_deps;

    let gleam_toml = project_root.join("gleam.toml");
    if !gleam_toml.is_file() { return Vec::new(); }
    let Ok(content) = std::fs::read_to_string(&gleam_toml) else { return Vec::new(); };
    let declared = parse_gleam_deps(&content);
    if declared.is_empty() { return Vec::new(); }

    let packages = project_root.join("build").join("packages");
    if !packages.is_dir() { return Vec::new(); }

    let mut roots = Vec::new();
    for dep in &declared {
        let dep_dir = packages.join(dep);
        if dep_dir.is_dir() {
            roots.push(ExternalDepRoot {
                module_path: dep.clone(),
                version: String::new(),
                root: dep_dir,
                ecosystem: "gleam",
                package_id: None,
            });
        }
    }
    debug!("Gleam: discovered {} external package roots", roots.len());
    roots
}

pub fn walk_gleam_external_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    let src = dep.root.join("src");
    let walk_root = if src.is_dir() { src } else { dep.root.clone() };
    walk_gleam_dir(&walk_root, &dep.root, dep, &mut out);
    out
}

fn walk_gleam_dir(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>) {
    walk_gleam_dir_bounded(dir, root, dep, out, 0);
}

fn walk_gleam_dir_bounded(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= MAX_WALK_DEPTH { return; }
    let Ok(entries) = std::fs::read_dir(dir) else { return; };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else { continue; };
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "test" | "tests") || name.starts_with('.') { continue; }
            }
            walk_gleam_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue; };
            if !name.ends_with(".gleam") { continue; }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            out.push(WalkedFile {
                relative_path: format!("ext:gleam:{}/{}", dep.module_path, rel_sub),
                absolute_path: path,
                language: "gleam",
            });
        }
    }
}
