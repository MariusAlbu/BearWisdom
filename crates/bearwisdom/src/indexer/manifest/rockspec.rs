// indexer/manifest/rockspec.rs — *.rockspec reader (Lua)

use std::path::Path;
use super::{ManifestData, ManifestKind, ManifestReader};

pub struct RockspecManifest;

impl ManifestReader for RockspecManifest {
    fn kind(&self) -> ManifestKind { ManifestKind::Rockspec }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let Ok(entries) = std::fs::read_dir(project_root) else {
            return None;
        };
        let rockspec = entries.flatten().find(|e| {
            e.path().extension().and_then(|x| x.to_str()) == Some("rockspec")
        })?;
        let content = std::fs::read_to_string(rockspec.path()).ok()?;
        let mut data = ManifestData::default();
        for name in parse_rockspec_deps(&content) {
            data.dependencies.insert(name);
        }
        Some(data)
    }
}

pub fn parse_rockspec_deps(content: &str) -> Vec<String> {
    let mut deps = Vec::new();
    let Some(start) = content.find("dependencies") else { return deps; };
    let rest = &content[start..];
    let Some(brace) = rest.find('{') else { return deps; };
    let rest = &rest[brace + 1..];
    let Some(end) = rest.find('}') else { return deps; };
    let block = &rest[..end];

    for part in block.split(',') {
        let trimmed = part.trim().trim_matches(|c: char| c == '\'' || c == '"' || c.is_whitespace());
        if trimmed.is_empty() { continue; }
        let name = trimmed.split(|c: char| c.is_whitespace() || c == '>' || c == '<' || c == '=' || c == '~')
            .next()
            .unwrap_or("")
            .trim();
        if !name.is_empty() && name != "lua" {
            deps.push(name.to_string());
        }
    }
    deps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rockspec() {
        let content = r#"
dependencies = {
  'lua == 5.1',
  'plenary.nvim',
}
"#;
        let deps = parse_rockspec_deps(content);
        assert_eq!(deps, vec!["plenary.nvim"]);
    }
}
