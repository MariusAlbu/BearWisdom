// ===========================================================================
// Manifest reader (migrated from indexer/manifest/pubspec.rs)
// ===========================================================================

use std::path::{Path, PathBuf};

use crate::ecosystem::manifest::{ManifestData, ManifestKind, ManifestReader, ReaderEntry};

pub struct PubspecManifest;

impl ManifestReader for PubspecManifest {
    fn kind(&self) -> ManifestKind {
        ManifestKind::Pubspec
    }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let entries = self.read_all(project_root);
        if entries.is_empty() {
            return None;
        }
        let mut data = ManifestData::default();
        for e in &entries {
            data.dependencies
                .extend(e.data.dependencies.iter().cloned());
        }
        Some(data)
    }

    fn read_all(&self, project_root: &Path) -> Vec<ReaderEntry> {
        let mut paths = Vec::new();
        collect_pubspec_files(project_root, &mut paths, 0);
        let mut out = Vec::new();
        for manifest_path in paths {
            let Ok(content) = std::fs::read_to_string(&manifest_path) else {
                continue;
            };
            let mut data = ManifestData::default();
            for name in parse_pubspec_deps(&content) {
                data.dependencies.insert(name);
            }
            let name = parse_pubspec_name(&content);
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

fn collect_pubspec_files(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 8 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if matches!(
                name.as_ref(),
                ".git" | ".dart_tool" | "build" | "node_modules" | "target" | "bin" | "obj"
            ) {
                continue;
            }
            collect_pubspec_files(&path, out, depth + 1);
        } else if entry.file_name() == "pubspec.yaml" {
            out.push(path);
        }
    }
}

pub(crate) fn parse_pubspec_name(content: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim_end();
        if trimmed.starts_with("name:") && !line.starts_with(' ') && !line.starts_with('\t') {
            let rest = trimmed["name:".len()..].trim();
            if !rest.is_empty() {
                return Some(rest.to_string());
            }
        }
    }
    None
}

pub fn parse_pubspec_deps(content: &str) -> Vec<String> {
    let mut packages = Vec::new();
    let mut in_deps = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        if !line.starts_with(' ') && !line.starts_with('\t') {
            in_deps = trimmed == "dependencies:" || trimmed == "dev_dependencies:";
            continue;
        }
        if !in_deps {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        if indent != 2 && !(line.starts_with('\t') && indent == 1) {
            continue;
        }
        if let Some(colon_pos) = trimmed.find(':') {
            let key = trimmed[..colon_pos].trim();
            if !key.is_empty()
                && key
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
            {
                packages.push(key.to_string());
            }
        }
    }
    packages
}
