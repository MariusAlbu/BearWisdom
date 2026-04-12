// indexer/manifest/opam.rs — *.opam reader (OCaml)

use std::path::Path;
use super::{ManifestData, ManifestKind, ManifestReader};

pub struct OpamManifest;

impl ManifestReader for OpamManifest {
    fn kind(&self) -> ManifestKind { ManifestKind::Opam }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let Ok(entries) = std::fs::read_dir(project_root) else {
            return None;
        };
        let opam_file = entries.flatten().find(|e| {
            e.path().extension().and_then(|x| x.to_str()) == Some("opam")
        })?;
        let content = std::fs::read_to_string(opam_file.path()).ok()?;
        let mut data = ManifestData::default();
        for name in parse_opam_depends(&content) {
            data.dependencies.insert(name);
        }
        Some(data)
    }
}

pub fn parse_opam_depends(content: &str) -> Vec<String> {
    let mut deps = Vec::new();
    let Some(start) = content.find("depends:") else { return deps; };
    let rest = &content[start + "depends:".len()..];
    let Some(bracket_start) = rest.find('[') else { return deps; };
    let rest = &rest[bracket_start + 1..];
    let Some(bracket_end) = rest.find(']') else { return deps; };
    let block = &rest[..bracket_end];

    for line in block.lines() {
        let trimmed = line.trim().trim_start_matches('"');
        if trimmed.is_empty() { continue; }
        // Package name is the first word, optionally quoted
        let name = trimmed.split(|c: char| c == '"' || c == ' ' || c == '{')
            .next()
            .unwrap_or("")
            .trim();
        if !name.is_empty()
            && name != "ocaml"
            && !name.starts_with("conf-")
            && name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            if !deps.contains(&name.to_string()) {
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
    fn parse_opam_deps() {
        let content = r#"
depends: [
  "dune" {>= "2.8.0"}
  "ocaml" {>= "4.08.1"}
  "conf-libpcre"
  "cohttp-lwt-unix"
  "core"
  "lwt"
  "yojson" {>= "1.6.0" < "2.0.0"}
]
"#;
        let deps = parse_opam_depends(content);
        assert!(deps.contains(&"cohttp-lwt-unix".to_string()));
        assert!(deps.contains(&"core".to_string()));
        assert!(deps.contains(&"lwt".to_string()));
        assert!(!deps.contains(&"ocaml".to_string()));
        assert!(!deps.contains(&"conf-libpcre".to_string()));
    }
}
