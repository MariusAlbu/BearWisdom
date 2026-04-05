// indexer/manifest/gemfile.rs — Gemfile reader

use std::path::Path;

use super::{ManifestData, ManifestKind, ManifestReader};

pub struct GemfileManifest;

impl ManifestReader for GemfileManifest {
    fn kind(&self) -> ManifestKind {
        ManifestKind::Gemfile
    }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let gemfile_path = project_root.join("Gemfile");
        if !gemfile_path.is_file() {
            return None;
        }
        let content = std::fs::read_to_string(&gemfile_path).ok()?;

        let mut data = ManifestData::default();
        for name in parse_gemfile_gems(&content) {
            data.dependencies.insert(name);
        }
        Some(data)
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Parse gem names from a Gemfile.
///
/// Handles:
///   `gem 'rails', '~> 7.0'`
///   `gem "devise"`
///   `gem 'sidekiq', require: false`
///
/// Returns the gem name only (first argument, without quotes or version).
pub fn parse_gemfile_gems(content: &str) -> Vec<String> {
    let mut gems = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let rest = if let Some(r) = trimmed.strip_prefix("gem ") {
            r.trim()
        } else {
            continue;
        };

        let name = if let Some(r) = rest.strip_prefix('\'') {
            r.split('\'').next().unwrap_or("").trim()
        } else if let Some(r) = rest.strip_prefix('"') {
            r.split('"').next().unwrap_or("").trim()
        } else {
            rest.split(|c: char| c == ',' || c.is_whitespace())
                .next()
                .unwrap_or("")
                .trim()
        };

        if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
            gems.push(name.to_string());
        }
    }
    gems
}
