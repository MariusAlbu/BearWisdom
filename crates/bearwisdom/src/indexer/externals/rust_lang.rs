// Rust / Cargo externals

use super::{find_first_subdir, ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;
use std::path::{Path, PathBuf};
use tracing::debug;

/// Rust Cargo → `discover_rust_externals` + `walk_rust_external_root`.
///
/// Cargo crates are stored in `~/.cargo/registry/src/<index-hash>/<crate>-<ver>/`.
/// Declared deps come from `Cargo.toml` `[dependencies]` / `[dev-dependencies]`.
/// Walk: `src/**/*.rs`, skipping `tests/`, `benches/`, `examples/`, `target/`.
pub struct RustExternalsLocator;

impl ExternalSourceLocator for RustExternalsLocator {
    fn ecosystem(&self) -> &'static str { "rust" }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_rust_externals(project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_rust_external_root(dep)
    }
}

pub fn discover_rust_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    use crate::indexer::manifest::cargo::parse_cargo_dependencies;

    let cargo_toml = project_root.join("Cargo.toml");
    if !cargo_toml.is_file() {
        return Vec::new();
    }
    let Ok(content) = std::fs::read_to_string(&cargo_toml) else {
        return Vec::new();
    };
    let declared = parse_cargo_dependencies(&content);
    if declared.is_empty() {
        return Vec::new();
    }

    let Some(registry_src) = cargo_registry_src() else {
        return Vec::new();
    };

    let Ok(entries) = std::fs::read_dir(&registry_src) else {
        return Vec::new();
    };
    let all_dirs: Vec<_> = entries.flatten()
        .filter(|e| e.path().is_dir())
        .collect();

    let mut roots = Vec::new();
    for crate_name in &declared {
        let prefix = format!("{crate_name}-");
        let mut matches: Vec<PathBuf> = all_dirs.iter()
            .filter(|e| {
                let n = e.file_name();
                let s = n.to_string_lossy();
                s.starts_with(&prefix) && s[prefix.len()..].chars().next().map_or(false, |c| c.is_ascii_digit())
            })
            .map(|e| e.path())
            .collect();
        matches.sort();
        if let Some(best) = matches.pop() {
            let version = best.file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_prefix(&prefix))
                .unwrap_or("")
                .to_string();
            roots.push(ExternalDepRoot {
                module_path: crate_name.clone(),
                version,
                root: best,
                ecosystem: "rust",
                package_id: None,
            });
        }
    }
    debug!("Rust: discovered {} external crate roots", roots.len());
    roots
}

fn cargo_registry_src() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("CARGO_HOME") {
        let src = PathBuf::from(home).join("registry").join("src");
        if src.is_dir() {
            return find_first_subdir(&src);
        }
    }
    let home = dirs::home_dir()?;
    let src = home.join(".cargo").join("registry").join("src");
    if src.is_dir() {
        return find_first_subdir(&src);
    }
    None
}

pub fn walk_rust_external_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    let src = dep.root.join("src");
    let walk_root = if src.is_dir() { src } else { dep.root.clone() };
    walk_rust_dir(&walk_root, &dep.root, dep, &mut out);
    out
}

fn walk_rust_dir(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>) {
    walk_rust_dir_bounded(dir, root, dep, out, 0);
}

fn walk_rust_dir_bounded(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= MAX_WALK_DEPTH { return; }
    let Ok(entries) = std::fs::read_dir(dir) else { return; };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else { continue; };
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "tests" | "test" | "benches" | "examples" | "target")
                    || name.starts_with('.')
                {
                    continue;
                }
            }
            walk_rust_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue; };
            if !name.ends_with(".rs") { continue; }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            out.push(WalkedFile {
                relative_path: format!("ext:rust:{}/{}", dep.module_path, rel_sub),
                absolute_path: path,
                language: "rust",
            });
        }
    }
}
