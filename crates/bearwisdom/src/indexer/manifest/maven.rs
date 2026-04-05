// indexer/manifest/maven.rs — pom.xml reader

use std::path::Path;

use super::{ManifestData, ManifestKind, ManifestReader};

pub struct MavenManifest;

impl ManifestReader for MavenManifest {
    fn kind(&self) -> ManifestKind {
        ManifestKind::Maven
    }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let mut pom_paths = Vec::new();
        collect_pom_files(project_root, &mut pom_paths, 0);

        if pom_paths.is_empty() {
            return None;
        }

        let mut data = ManifestData::default();
        for path in &pom_paths {
            let content = match std::fs::read_to_string(path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            for group_id in parse_pom_xml_dependencies(&content) {
                data.dependencies.insert(group_id);
            }
        }
        Some(data)
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn collect_pom_files(dir: &Path, out: &mut Vec<std::path::PathBuf>, depth: usize) {
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
                ".git" | "target" | "build" | "node_modules" | ".gradle" | "bin" | "obj"
            ) {
                continue;
            }
            collect_pom_files(&path, out, depth + 1);
        } else {
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();
            if name == "pom.xml" {
                out.push(path);
            }
        }
    }
}

/// Parse `<dependency><groupId>...</groupId><artifactId>...</artifactId>` from pom.xml.
///
/// Returns a list of groupId strings (e.g., "org.springframework", "com.google.guava").
/// Lightweight line-based parsing — no XML library needed.
pub fn parse_pom_xml_dependencies(content: &str) -> Vec<String> {
    let mut group_ids = Vec::new();
    let mut in_dependency = false;
    let mut current_group_id: Option<String> = None;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.contains("<dependency>") {
            in_dependency = true;
            current_group_id = None;
            continue;
        }
        if trimmed.contains("</dependency>") {
            if let Some(gid) = current_group_id.take() {
                group_ids.push(gid);
            }
            in_dependency = false;
            continue;
        }

        if !in_dependency {
            continue;
        }

        if let Some(value) = extract_xml_text(trimmed, "groupId") {
            current_group_id = Some(value);
        }
    }

    group_ids
}

/// Extract the text content of a simple XML element on a single line.
/// e.g., `<groupId>org.springframework</groupId>` → `Some("org.springframework")`
pub fn extract_xml_text(line: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = line.find(&open)?;
    let after_open = &line[start + open.len()..];
    let end = after_open.find(&close)?;
    let value = after_open[..end].trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}
