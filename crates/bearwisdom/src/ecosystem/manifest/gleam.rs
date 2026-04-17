// indexer/manifest/gleam.rs — gleam.toml reader

use std::path::Path;
use super::{ManifestData, ManifestKind, ManifestReader};

pub struct GleamManifest;

impl ManifestReader for GleamManifest {
    fn kind(&self) -> ManifestKind { ManifestKind::Gleam }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let gleam_toml = project_root.join("gleam.toml");
        if !gleam_toml.is_file() { return None; }
        let content = std::fs::read_to_string(&gleam_toml).ok()?;
        let mut data = ManifestData::default();
        for name in parse_gleam_deps(&content) {
            data.dependencies.insert(name);
        }
        Some(data)
    }
}

pub fn parse_gleam_deps(content: &str) -> Vec<String> {
    let mut deps = Vec::new();
    let mut in_deps = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "[dependencies]" || trimmed == "[dev-dependencies]" {
            in_deps = true;
            continue;
        }
        if trimmed.starts_with('[') {
            in_deps = false;
            continue;
        }
        if !in_deps { continue; }
        if let Some(eq) = trimmed.find('=') {
            let name = trimmed[..eq].trim();
            if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
                deps.push(name.to_string());
            }
        }
    }
    deps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_gleam_toml() {
        let content = r#"
name = "lustre"
version = "5.6.0"

[dependencies]
gleam_erlang = ">= 1.0.0 and < 2.0.0"
gleam_json = ">= 1.0.0 and < 4.0.0"

[dev-dependencies]
birdie = "~> 1.0"
"#;
        let deps = parse_gleam_deps(content);
        assert_eq!(deps, vec!["gleam_erlang", "gleam_json", "birdie"]);
    }
}
