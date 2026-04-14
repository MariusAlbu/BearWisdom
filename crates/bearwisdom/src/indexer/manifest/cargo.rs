// indexer/manifest/cargo.rs — Cargo.toml reader

use std::path::{Path, PathBuf};

use super::{ManifestData, ManifestKind, ManifestReader, ReaderEntry};

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

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

pub(super) fn collect_cargo_tomls(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 8 {
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

/// Parse crate names from `[dependencies]` and `[dev-dependencies]` sections.
///
/// TOML parsing is done line-by-line to avoid pulling in a full TOML crate.
/// We only need crate names (keys), not version strings.
///
/// Handles:
///   `serde = "1"`
///   `tokio = { version = "1", features = ["full"] }`
///   `my-crate.workspace = true`
pub fn parse_cargo_dependencies(content: &str) -> Vec<String> {
    let mut crates = Vec::new();
    let mut in_dep_section = false;

    for line in content.lines() {
        let trimmed = line.trim();

        // Detect section headers.
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

        if !in_dep_section {
            continue;
        }

        // Skip blank lines and comments.
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Extract the crate name (the key before `=`).
        // Keys may contain hyphens and underscores but not spaces.
        if let Some(eq_pos) = trimmed.find('=') {
            let key = trimmed[..eq_pos]
                .trim()
                // Strip dotted suffixes like `tokio.workspace`
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

/// Parse the `[package].name` field from a Cargo.toml.
///
/// Returns `None` for workspace-root manifests that declare no `[package]`
/// section (pure `[workspace]` manifests).
fn parse_cargo_package_name(content: &str) -> Option<String> {
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
        // `name = "crate-name"` — take the key before `=`, value between quotes.
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
