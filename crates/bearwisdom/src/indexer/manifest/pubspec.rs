// indexer/manifest/pubspec.rs — pubspec.yaml reader (Dart/Flutter)

use std::path::Path;

use super::{ManifestData, ManifestKind, ManifestReader};

pub struct PubspecManifest;

impl ManifestReader for PubspecManifest {
    fn kind(&self) -> ManifestKind {
        ManifestKind::Pubspec
    }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let pubspec_path = project_root.join("pubspec.yaml");
        if !pubspec_path.is_file() {
            return None;
        }
        let content = std::fs::read_to_string(&pubspec_path).ok()?;

        let mut data = ManifestData::default();
        for name in parse_pubspec_deps(&content) {
            data.dependencies.insert(name);
        }
        Some(data)
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Parse package names from a `pubspec.yaml` file.
///
/// YAML structure (relevant sections):
/// ```yaml
/// dependencies:
///   flutter:
///     sdk: flutter
///   http: ^0.13.0
///   provider: ^6.0.0
///
/// dev_dependencies:
///   flutter_test:
///     sdk: flutter
///   build_runner: ^2.0.0
/// ```
///
/// Line-based parsing — no YAML crate required.
/// Keys at two-space indent inside `dependencies:` / `dev_dependencies:` sections
/// are package names. The `sdk:` sub-key lines are not package names.
pub fn parse_pubspec_deps(content: &str) -> Vec<String> {
    let mut packages = Vec::new();
    let mut in_deps = false;

    for line in content.lines() {
        // Skip comment lines.
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }

        // Detect top-level section headers (no leading spaces).
        if !line.starts_with(' ') && !line.starts_with('\t') {
            in_deps = trimmed == "dependencies:" || trimmed == "dev_dependencies:";
            continue;
        }

        if !in_deps {
            continue;
        }

        // A package entry is a key at the first indentation level (2 spaces or 1 tab).
        // Keys are YAML map entries: `  package_name:` or `  package_name: version`.
        // Sub-keys like `    sdk: flutter` are indented further — skip them.
        let indent = line.len() - line.trim_start().len();
        if indent != 2 && !(line.starts_with('\t') && indent == 1) {
            // Deeper indentation = sub-key of a package entry (e.g., `sdk: flutter`).
            continue;
        }

        // The key is everything before the first `:`.
        if let Some(colon_pos) = trimmed.find(':') {
            let key = trimmed[..colon_pos].trim();
            if !key.is_empty()
                && key.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-')
            {
                packages.push(key.to_string());
            }
        }
    }

    packages
}
