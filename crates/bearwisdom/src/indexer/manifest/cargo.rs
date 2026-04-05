// indexer/manifest/cargo.rs — Cargo.toml reader

use std::path::Path;

use super::{ManifestData, ManifestKind, ManifestReader};

pub struct CargoManifest;

impl ManifestReader for CargoManifest {
    fn kind(&self) -> ManifestKind {
        ManifestKind::Cargo
    }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let mut paths = Vec::new();
        collect_cargo_tomls(project_root, &mut paths, 0);

        if paths.is_empty() {
            return None;
        }

        let mut data = ManifestData::default();
        for path in &paths {
            let content = match std::fs::read_to_string(path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            for name in parse_cargo_dependencies(&content) {
                data.dependencies.insert(name);
            }
        }
        Some(data)
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

pub(super) fn collect_cargo_tomls(dir: &Path, out: &mut Vec<std::path::PathBuf>, depth: usize) {
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
