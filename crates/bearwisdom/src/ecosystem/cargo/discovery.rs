// ===========================================================================
// Discovery — Cargo.lock → ~/.cargo/registry/src/<index>/<name>-<ver>/
// ===========================================================================

use std::path::{Path, PathBuf};

use tracing::debug;

use super::manifest::parse_cargo_dependencies;
use super::LEGACY_ECOSYSTEM_TAG;
use crate::ecosystem::externals::ExternalDepRoot;

#[derive(Debug, Clone)]
pub(super) struct CargoLockEntry {
    pub(super) name: String,
    pub(super) version: String,
}

/// Parse `[[package]]` entries from `Cargo.lock`. Only returns packages with
/// `source = "registry+..."` — workspace members and git deps are omitted.
pub(super) fn parse_cargo_lock(content: &str) -> Vec<CargoLockEntry> {
    let mut entries = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current_version: Option<String> = None;
    let mut current_is_registry = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "[[package]]" {
            if current_is_registry {
                if let (Some(name), Some(version)) = (current_name.take(), current_version.take()) {
                    entries.push(CargoLockEntry { name, version });
                }
            } else {
                current_name = None;
                current_version = None;
            }
            current_is_registry = false;
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some(eq) = trimmed.find(" = ") else {
            continue;
        };
        let key = trimmed[..eq].trim();
        let rest = trimmed[eq + 3..].trim();
        let value = rest.trim_matches('"');
        match key {
            "name" => {
                current_name = Some(value.to_string());
            }
            "version" => {
                current_version = Some(value.to_string());
            }
            "source" => {
                current_is_registry = value.starts_with("registry+");
            }
            _ => {}
        }
    }
    if current_is_registry {
        if let (Some(name), Some(version)) = (current_name, current_version) {
            entries.push(CargoLockEntry { name, version });
        }
    }
    entries
}

fn find_cargo_lock(start: &Path) -> Option<PathBuf> {
    let mut current = start;
    for _ in 0..8 {
        let lock = current.join("Cargo.lock");
        if lock.is_file() {
            return Some(lock);
        }
        current = current.parent()?;
    }
    None
}

fn find_cargo_lock_descend(start: &Path) -> Option<PathBuf> {
    find_cargo_lock_descend_bounded(start, 0)
}

fn find_cargo_lock_descend_bounded(dir: &Path, depth: u8) -> Option<PathBuf> {
    if depth > 2 {
        return None;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return None;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.file_name().and_then(|n| n.to_str()) == Some("Cargo.lock") {
            return Some(path);
        }
        if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "target" | ".git" | "node_modules") || name.starts_with('.') {
                    continue;
                }
            }
            if let Some(found) = find_cargo_lock_descend_bounded(&path, depth + 1) {
                return Some(found);
            }
        }
    }
    None
}

fn cargo_registry_src_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let src_root = if let Ok(home) = std::env::var("CARGO_HOME") {
        PathBuf::from(home).join("registry").join("src")
    } else if let Some(home) = dirs::home_dir() {
        home.join(".cargo").join("registry").join("src")
    } else {
        return dirs;
    };
    if !src_root.is_dir() {
        return dirs;
    }
    if let Ok(entries) = std::fs::read_dir(&src_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path)
            }
        }
    }
    dirs
}

pub(crate) fn split_crate_dir_name(s: &str) -> Option<(String, String)> {
    let bytes = s.as_bytes();
    let mut i = s.len();
    while let Some(pos) = s[..i].rfind('-') {
        if bytes.get(pos + 1).map_or(false, |b| b.is_ascii_digit()) {
            return Some((s[..pos].to_string(), s[pos + 1..].to_string()));
        }
        i = pos;
    }
    None
}

pub(super) fn discover_cargo_roots(project_root: &Path) -> Vec<ExternalDepRoot> {
    let lock_path = find_cargo_lock(project_root).or_else(|| find_cargo_lock_descend(project_root));

    let packages: Vec<CargoLockEntry> = if let Some(ref lp) = lock_path {
        if let Ok(content) = std::fs::read_to_string(lp) {
            let parsed = parse_cargo_lock(&content);
            if !parsed.is_empty() {
                debug!(
                    "Rust: loaded {} packages from {}",
                    parsed.len(),
                    lp.display()
                );
                parsed
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    let use_fallback = packages.is_empty();
    let toml_names: Vec<String> = if use_fallback {
        let cargo_toml = project_root.join("Cargo.toml");
        if !cargo_toml.is_file() {
            return Vec::new();
        }
        match std::fs::read_to_string(&cargo_toml) {
            Ok(content) => {
                let deps = parse_cargo_dependencies(&content);
                if deps.is_empty() {
                    return Vec::new();
                }
                debug!(
                    "Rust: no Cargo.lock; {} declared deps from Cargo.toml",
                    deps.len()
                );
                deps
            }
            Err(_) => return Vec::new(),
        }
    } else {
        Vec::new()
    };

    let src_dirs = cargo_registry_src_dirs();
    if src_dirs.is_empty() {
        debug!("Rust: no ~/.cargo/registry/src found; skipping");
        return Vec::new();
    }

    let mut all_crate_dirs: Vec<PathBuf> = Vec::new();
    for src_dir in &src_dirs {
        if let Ok(entries) = std::fs::read_dir(src_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    all_crate_dirs.push(path)
                }
            }
        }
    }

    let mut roots = Vec::new();

    if use_fallback {
        for crate_name in &toml_names {
            let prefix = format!("{crate_name}-");
            let mut matches: Vec<PathBuf> = all_crate_dirs
                .iter()
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .map(|s| {
                            s.starts_with(&prefix)
                                && s[prefix.len()..]
                                    .chars()
                                    .next()
                                    .map_or(false, |c| c.is_ascii_digit())
                        })
                        .unwrap_or(false)
                })
                .cloned()
                .collect();
            matches.sort();
            if let Some(best) = matches.pop() {
                let version = best
                    .file_name()
                    .and_then(|n| n.to_str())
                    .and_then(|n| n.strip_prefix(&prefix))
                    .unwrap_or("")
                    .to_string();
                roots.push(ExternalDepRoot {
                    module_path: crate_name.clone(),
                    version,
                    root: best,
                    ecosystem: LEGACY_ECOSYSTEM_TAG,
                    package_id: None,
                    requested_imports: Vec::new(),
                });
            }
        }
    } else {
        let mut dir_index: std::collections::HashMap<(String, String), PathBuf> =
            std::collections::HashMap::with_capacity(all_crate_dirs.len());

        for path in &all_crate_dirs {
            let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if let Some((name, version)) = split_crate_dir_name(dir_name) {
                dir_index
                    .entry((name, version))
                    .or_insert_with(|| path.clone());
            }
        }

        for entry in &packages {
            let key = (entry.name.clone(), entry.version.clone());
            let under_key = (entry.name.replace('-', "_"), entry.version.clone());
            let found = dir_index
                .get(&key)
                .or_else(|| dir_index.get(&under_key))
                .cloned();
            if let Some(crate_root) = found {
                roots.push(ExternalDepRoot {
                    module_path: entry.name.clone(),
                    version: entry.version.clone(),
                    root: crate_root,
                    ecosystem: LEGACY_ECOSYSTEM_TAG,
                    package_id: None,
                    requested_imports: Vec::new(),
                });
            }
        }
    }

    // Register each crate root's project-declared feature set so the cfg-gated
    // module walk can bound feature-packed crates (notably `windows`). Crates
    // absent from the map get no registration — the walk fails open and indexes
    // every module, matching the pre-gate behaviour.
    let crate_features = super::features::collect_crate_features(project_root);
    for root in &roots {
        if let Some(features) = crate_features.get(&root.module_path) {
            super::features::register_root_features(&root.root, features.clone());
        }
    }

    debug!("Rust: discovered {} external crate roots", roots.len());
    roots
}
