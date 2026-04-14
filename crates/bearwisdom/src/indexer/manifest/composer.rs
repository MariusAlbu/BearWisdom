// indexer/manifest/composer.rs — composer.json reader

use std::path::{Path, PathBuf};

use super::{ManifestData, ManifestKind, ManifestReader, ReaderEntry};

pub struct ComposerManifest;

impl ManifestReader for ComposerManifest {
    fn kind(&self) -> ManifestKind {
        ManifestKind::Composer
    }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let entries = self.read_all(project_root);
        if entries.is_empty() {
            return None;
        }
        let mut data = ManifestData::default();
        for e in &entries {
            data.dependencies.extend(e.data.dependencies.iter().cloned());
        }
        Some(data)
    }

    fn read_all(&self, project_root: &Path) -> Vec<ReaderEntry> {
        let mut paths = Vec::new();
        collect_composer_files(project_root, &mut paths, 0);

        let mut out = Vec::new();
        for manifest_path in paths {
            let Ok(content) = std::fs::read_to_string(&manifest_path) else { continue };

            let mut data = ManifestData::default();
            let (name, deps) = parse_composer_json(&content);
            for pkg in deps {
                data.dependencies.insert(pkg);
            }

            let package_dir = manifest_path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| project_root.to_path_buf());

            out.push(ReaderEntry {
                package_dir,
                manifest_path,
                data,
                name,
            });
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn collect_composer_files(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 6 {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if matches!(
                name.as_ref(),
                ".git" | "vendor" | "node_modules" | "target" | "bin" | "obj"
            ) {
                continue;
            }
            collect_composer_files(&path, out, depth + 1);
        } else if entry.file_name() == "composer.json" {
            out.push(path);
        }
    }
}

/// Parse a composer.json into (name, dependency-names).
///
/// Skips platform requirements (`php`, `ext-*`, `lib-*`).
fn parse_composer_json(content: &str) -> (Option<String>, Vec<String>) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(content) else {
        return (None, Vec::new());
    };
    let Some(obj) = value.as_object() else {
        return (None, Vec::new());
    };

    let name = obj
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let mut packages = Vec::new();
    for key in &["require", "require-dev"] {
        if let Some(serde_json::Value::Object(deps)) = obj.get(*key) {
            for pkg_name in deps.keys() {
                if pkg_name == "php"
                    || pkg_name.starts_with("ext-")
                    || pkg_name.starts_with("lib-")
                {
                    continue;
                }
                if !pkg_name.is_empty() {
                    packages.push(pkg_name.clone());
                }
            }
        }
    }
    (name, packages)
}

/// Legacy helper kept for external consumers.
pub fn parse_composer_json_deps(content: &str) -> Vec<String> {
    parse_composer_json(content).1
}
