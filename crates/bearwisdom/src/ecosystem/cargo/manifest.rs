// ===========================================================================
// Manifest reader — migrated from indexer/manifest/cargo.rs
// ===========================================================================

use std::path::{Path, PathBuf};

use crate::ecosystem::manifest::{ManifestData, ManifestKind, ManifestReader, ReaderEntry};

/// `CargoManifest` reads `Cargo.toml` + `Cargo.lock` per-package during
/// `ProjectContext` building. Still lives as a `ManifestReader` impl so the
/// existing `manifest::all_readers()` registry continues to dispatch it.
/// Phase 4 (ProjectContext wiring) collapses this path into an Ecosystem-
/// native manifest flow.
pub struct CargoManifest;

impl ManifestReader for CargoManifest {
    fn kind(&self) -> ManifestKind {
        ManifestKind::Cargo
    }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let entries = self.read_all(project_root);
        if entries.is_empty() { return None }
        let mut data = ManifestData::default();
        for e in &entries {
            data.dependencies.extend(e.data.dependencies.iter().cloned());
        }
        Some(data)
    }

    fn read_all(&self, project_root: &Path) -> Vec<ReaderEntry> {
        let mut paths = Vec::new();
        collect_cargo_tomls(project_root, &mut paths, 0);

        let mut out = Vec::new();
        for manifest_path in paths {
            let Ok(content) = std::fs::read_to_string(&manifest_path) else { continue };

            let mut data = ManifestData::default();
            for name in parse_cargo_dependencies(&content) {
                data.dependencies.insert(name);
            }
            for key in parse_cargo_path_dependencies(&content) {
                if !data.project_refs.contains(&key) {
                    data.project_refs.push(key);
                }
            }

            let name = parse_cargo_package_name(&content);
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

fn collect_cargo_tomls(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 8 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if matches!(
                name.as_ref(),
                "target" | ".git" | "node_modules" | "bin" | "obj" | ".cargo"
            ) { continue }
            collect_cargo_tomls(&path, out, depth + 1);
        } else if entry.file_name() == "Cargo.toml" {
            out.push(path);
        }
    }
}

/// Parse crate names from `[dependencies]` + `[dev-dependencies]` +
/// `[build-dependencies]` + `[workspace.dependencies]` sections.
///
/// Line-by-line scan — avoids a full TOML dependency. Handles
/// `serde = "1"`, `tokio = { ... }`, `foo.workspace = true`.
pub fn parse_cargo_dependencies(content: &str) -> Vec<String> {
    let mut crates = Vec::new();
    let mut in_dep_section = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_dep_section = matches!(
                trimmed,
                "[dependencies]"
                    | "[dev-dependencies]"
                    | "[build-dependencies]"
                    | "[workspace.dependencies]"
            );
            continue;
        }
        if !in_dep_section { continue }
        if trimmed.is_empty() || trimmed.starts_with('#') { continue }

        if let Some(eq_pos) = trimmed.find('=') {
            let key = trimmed[..eq_pos]
                .trim()
                .split('.')
                .next()
                .unwrap_or("")
                .trim();
            if !key.is_empty()
                && key.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
            {
                crates.push(key.to_string());
            }
        }
    }
    crates
}

/// Parse sibling-workspace crate names from `path = "..."` dependency entries.
/// Each entry yields the dep KEY (the Cargo-side name), not the target crate's
/// `[package].name` — the key is what appears in `use foo::...` source code.
pub fn parse_cargo_path_dependencies(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_dep_section = false;
    let mut pending_key: Option<String> = None;
    let mut pending_table = String::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_dep_section = matches!(
                trimmed,
                "[dependencies]"
                    | "[dev-dependencies]"
                    | "[build-dependencies]"
                    | "[workspace.dependencies]"
            );
            pending_key = None;
            pending_table.clear();
            continue;
        }
        if !in_dep_section { continue }
        if trimmed.is_empty() || trimmed.starts_with('#') { continue }

        if let Some(key) = pending_key.clone() {
            pending_table.push(' ');
            pending_table.push_str(trimmed);
            if trimmed.contains('}') {
                if pending_table.contains("path") && pending_table.contains('=') {
                    if !out.contains(&key) { out.push(key) }
                }
                pending_key = None;
                pending_table.clear();
            }
            continue;
        }

        let Some(eq) = trimmed.find('=') else { continue };
        let key = trimmed[..eq]
            .trim()
            .split('.')
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        if key.is_empty()
            || !key.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        { continue }
        let value = trimmed[eq + 1..].trim();
        if value.starts_with('{') && value.ends_with('}') {
            if value.contains("path") && value.contains('=') {
                if !out.contains(&key) { out.push(key) }
            }
            continue;
        }
        if value.starts_with('{') {
            pending_key = Some(key);
            pending_table.push_str(value);
            continue;
        }
    }
    out
}

pub(super) fn parse_cargo_package_name(content: &str) -> Option<String> {
    let mut in_package = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_package = trimmed == "[package]";
            continue;
        }
        if !in_package { continue }
        if let Some(rest) = trimmed.strip_prefix("name") {
            let rest = rest.trim_start();
            let Some(rest) = rest.strip_prefix('=') else { continue };
            let rest = rest.trim();
            let Some(rest) = rest.strip_prefix('"') else { continue };
            let Some(end) = rest.find('"') else { continue };
            return Some(rest[..end].to_string());
        }
    }
    None
}
