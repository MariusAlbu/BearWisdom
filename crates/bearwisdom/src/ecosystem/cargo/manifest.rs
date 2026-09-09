// ===========================================================================
// Manifest reader — migrated from indexer/manifest/cargo.rs
// ===========================================================================

use std::path::{Path, PathBuf};

#[path = "module_manifest.rs"]
mod module_manifest;
#[path = "registry_modules.rs"]
mod registry_modules;

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
        if entries.is_empty() {
            return None;
        }
        let mut data = ManifestData::default();
        for e in &entries {
            data.module_packages
                .extend(e.data.module_packages.iter().cloned());
            data.dependencies
                .extend(e.data.dependencies.iter().cloned());
        }
        Some(data)
    }

    fn read_all(&self, project_root: &Path) -> Vec<ReaderEntry> {
        let mut paths = Vec::new();
        collect_cargo_tomls(project_root, &mut paths, 0);

        let mut entries: Vec<_> = paths
            .into_iter()
            .filter_map(|path| module_manifest::entry(path, project_root))
            .collect();
        registry_modules::extend(
            &mut entries,
            project_root,
            &super::discovery::cargo_registry_src_dirs(),
        );
        entries
    }
}

fn collect_cargo_tomls(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
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
                "target" | ".git" | "node_modules" | "bin" | "obj" | ".cargo"
            ) {
                continue;
            }
            collect_cargo_tomls(&path, out, depth + 1);
        } else if entry.file_name() == "Cargo.toml" {
            out.push(path);
        }
    }
}

/// True for any TOML table header that declares Cargo dependencies. Covers:
/// `[dependencies]`, `[dev-dependencies]`, `[build-dependencies]`,
/// `[workspace.dependencies]`, `[workspace.dev-dependencies]`, and the
/// per-target form `[target.'cfg(...)'.dependencies]` /
/// `[target."cfg(...)".dev-dependencies]` / `[target.x86_64-pc-windows.dependencies]`
/// — anything `.dependencies]` / `.dev-dependencies]` / `.build-dependencies]`
/// terminated.
fn is_cargo_dependency_section(trimmed: &str) -> bool {
    matches!(
        trimmed,
        "[dependencies]"
            | "[dev-dependencies]"
            | "[build-dependencies]"
            | "[workspace.dependencies]"
            | "[workspace.dev-dependencies]"
            | "[workspace.build-dependencies]"
    ) || (trimmed.starts_with("[target.")
        && (trimmed.ends_with(".dependencies]")
            || trimmed.ends_with(".dev-dependencies]")
            || trimmed.ends_with(".build-dependencies]")))
}

/// True for sub-table-form headers `[dependencies.crate]`, `[dev-dependencies.crate]`,
/// `[workspace.dependencies.crate]`, `[target.'cfg(...)'.dependencies.crate]`.
/// Returns the crate name extracted from the header.
fn cargo_subtable_dep_name(trimmed: &str) -> Option<&str> {
    let body = trimmed.strip_prefix('[')?.strip_suffix(']')?;
    for marker in [
        "workspace.dependencies.",
        "workspace.dev-dependencies.",
        "workspace.build-dependencies.",
        "dev-dependencies.",
        "build-dependencies.",
        "dependencies.",
    ] {
        if let Some(name) = body.rsplit_once(marker).map(|(_, n)| n) {
            if !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
            {
                return Some(name);
            }
        }
    }
    None
}

/// Parse crate names from every Cargo dependency table. See `is_cargo_dependency_section`.
///
/// Line-by-line scan — avoids a full TOML dependency. Handles
/// `serde = "1"`, `tokio = { ... }`, `foo.workspace = true`, plus
/// the sub-table form `[dependencies.foo]\nversion = "1"`.
/// Parse Cargo dependency renames: an inline-table dep entry carrying an
/// explicit `package = "X"` key is a rename. `alias = { package = "X", .. }`
/// yields `(alias, "X")`; plain entries use the key as the crate name.
pub fn parse_cargo_dep_renames(content: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut in_dep_section = false;
    let mut pending_key: Option<String> = None;
    let mut pending_table = String::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_dep_section = is_cargo_dependency_section(trimmed);
            pending_key = None;
            pending_table.clear();
            continue;
        }
        if !in_dep_section || trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(key) = pending_key.clone() {
            pending_table.push(' ');
            pending_table.push_str(trimmed);
            if trimmed.contains('}') {
                if let Some(pkg) = cargo_table_package_field(&pending_table) {
                    out.push((key, pkg));
                }
                pending_key = None;
                pending_table.clear();
            }
            continue;
        }
        let Some(eq) = trimmed.find('=') else {
            continue;
        };
        let key = trimmed[..eq]
            .trim()
            .split('.')
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        if key.is_empty()
            || !key
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            continue;
        }
        let value = trimmed[eq + 1..].trim();
        if value.starts_with('{') && value.ends_with('}') {
            if let Some(pkg) = cargo_table_package_field(value) {
                out.push((key, pkg));
            }
            continue;
        }
        if value.starts_with('{') {
            pending_key = Some(key);
            pending_table.push_str(value);
        }
    }
    out
}

/// The `package = "X"` value inside an inline-table dependency body, if any.
fn cargo_table_package_field(table: &str) -> Option<String> {
    let idx = table.find("package")?;
    let after = table[idx + "package".len()..].trim_start();
    let after = after.strip_prefix('=')?.trim_start();
    let after = after.strip_prefix('"')?;
    let end = after.find('"')?;
    Some(after[..end].to_string())
}

pub fn parse_cargo_dependencies(content: &str) -> Vec<String> {
    let mut crates = Vec::new();
    let mut in_dep_section = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            // Sub-table form: the crate name is in the header itself, and the
            // body holds version/feature keys (which we skip — only the crate
            // matters here). The header sets in_dep_section=false so the
            // body's `version = "1"` line doesn't get treated as a flat dep.
            if let Some(name) = cargo_subtable_dep_name(trimmed) {
                crates.push(name.to_string());
                in_dep_section = false;
                continue;
            }
            in_dep_section = is_cargo_dependency_section(trimmed);
            continue;
        }
        if !in_dep_section {
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if let Some(eq_pos) = trimmed.find('=') {
            let key = trimmed[..eq_pos]
                .trim()
                .split('.')
                .next()
                .unwrap_or("")
                .trim();
            if !key.is_empty()
                && key
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
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
            in_dep_section = is_cargo_dependency_section(trimmed);
            pending_key = None;
            pending_table.clear();
            continue;
        }
        if !in_dep_section {
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if let Some(key) = pending_key.clone() {
            pending_table.push(' ');
            pending_table.push_str(trimmed);
            if trimmed.contains('}') {
                if pending_table.contains("path") && pending_table.contains('=') {
                    if !out.contains(&key) {
                        out.push(key)
                    }
                }
                pending_key = None;
                pending_table.clear();
            }
            continue;
        }

        let Some(eq) = trimmed.find('=') else {
            continue;
        };
        let key = trimmed[..eq]
            .trim()
            .split('.')
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        if key.is_empty()
            || !key
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            continue;
        }
        let value = trimmed[eq + 1..].trim();
        if value.starts_with('{') && value.ends_with('}') {
            if value.contains("path") && value.contains('=') {
                if !out.contains(&key) {
                    out.push(key)
                }
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
        if !in_package {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("name") {
            let rest = rest.trim_start();
            let Some(rest) = rest.strip_prefix('=') else {
                continue;
            };
            let rest = rest.trim();
            let Some(rest) = rest.strip_prefix('"') else {
                continue;
            };
            let Some(end) = rest.find('"') else { continue };
            return Some(rest[..end].to_string());
        }
    }
    None
}
