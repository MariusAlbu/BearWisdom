// indexer/manifest/composer.rs — composer.json reader

use std::path::Path;

use super::{ManifestData, ManifestKind, ManifestReader};

pub struct ComposerManifest;

impl ManifestReader for ComposerManifest {
    fn kind(&self) -> ManifestKind {
        ManifestKind::Composer
    }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let composer_path = project_root.join("composer.json");
        if !composer_path.is_file() {
            return None;
        }
        let content = std::fs::read_to_string(&composer_path).ok()?;

        let mut data = ManifestData::default();
        for name in parse_composer_json_deps(&content) {
            data.dependencies.insert(name);
        }
        Some(data)
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Extract package names from a composer.json file's `require` and `require-dev` objects.
///
/// Uses `serde_json` for parsing since it's already a workspace dependency.
/// Skips the `php` and `ext-*` entries (platform requirements, not packages).
pub fn parse_composer_json_deps(content: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(content) else {
        return Vec::new();
    };
    let Some(obj) = value.as_object() else {
        return Vec::new();
    };

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
    packages
}
