// Zig / zon externals

use super::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;
use std::path::Path;
use tracing::debug;

/// Zig zon → `discover_zig_externals` + `walk_zig_external_root`.
///
/// Zig fetches deps to `.zig-cache/p/<hash>/`. Directory names are content hashes,
/// not package names. We match by reading `build.zig.zon` inside each hash dir.
/// Declared deps come from `build.zig.zon` `.dependencies` section.
/// Walk: `*.zig` files, skipping `tests/`, `test/`, `zig-cache/`.
pub struct ZigExternalsLocator;

impl ExternalSourceLocator for ZigExternalsLocator {
    fn ecosystem(&self) -> &'static str { "zig" }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_zig_externals(project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_zig_external_root(dep)
    }
}

pub fn discover_zig_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    use crate::indexer::manifest::zig_zon::parse_zig_zon_deps;

    let zon = project_root.join("build.zig.zon");
    if !zon.is_file() { return Vec::new(); }
    let Ok(content) = std::fs::read_to_string(&zon) else { return Vec::new(); };
    let declared = parse_zig_zon_deps(&content);
    if declared.is_empty() { return Vec::new(); }

    // Zig fetches deps to .zig-cache/p/<hash>/ — directory names are hashes,
    // not package names. We look for build.zig.zon inside each to match names.
    let cache = project_root.join(".zig-cache").join("p");
    if !cache.is_dir() { return Vec::new(); }

    let Ok(entries) = std::fs::read_dir(&cache) else { return Vec::new(); };
    let mut roots = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() { continue; }
        let zon_path = path.join("build.zig.zon");
        if let Ok(zon_content) = std::fs::read_to_string(&zon_path) {
            if let Some(name) = extract_zig_zon_name(&zon_content) {
                if declared.iter().any(|d| d == &name) {
                    roots.push(ExternalDepRoot {
                        module_path: name,
                        version: String::new(),
                        root: path,
                        ecosystem: "zig",
                    });
                }
            }
        }
    }
    debug!("Zig: discovered {} external package roots", roots.len());
    roots
}

fn extract_zig_zon_name(content: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with(".name") {
            // .name = .dep_name, or .name = "dep_name",
            let rest = trimmed.splitn(2, '=').nth(1)?.trim();
            let name = rest.trim_start_matches('.').trim_matches(|c: char| c == ',' || c == '"' || c.is_whitespace());
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    None
}

pub fn walk_zig_external_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_zig_dir(&dep.root, &dep.root, dep, &mut out);
    out
}

fn walk_zig_dir(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>) {
    walk_zig_dir_bounded(dir, root, dep, out, 0);
}

fn walk_zig_dir_bounded(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= MAX_WALK_DEPTH { return; }
    let Ok(entries) = std::fs::read_dir(dir) else { return; };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else { continue; };
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "test" | "tests" | "zig-cache") || name.starts_with('.') { continue; }
            }
            walk_zig_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue; };
            if !name.ends_with(".zig") { continue; }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            out.push(WalkedFile {
                relative_path: format!("ext:zig:{}/{}", dep.module_path, rel_sub),
                absolute_path: path,
                language: "zig",
            });
        }
    }
}
