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

/// A fully-qualified Maven coordinate extracted from a pom.xml dependency.
/// Needed by the externals discovery pass to locate the `-sources.jar` on
/// disk — only `group_id` isn't sufficient because the local repository
/// layout is `groupId.replace('.', '/') / artifactId / version / file`.
#[derive(Debug, Clone)]
pub struct MavenCoord {
    pub group_id: String,
    pub artifact_id: String,
    /// None when the pom uses version property resolution (`${spring.version}`)
    /// that our line parser doesn't handle. Callers can probe the `versions/`
    /// directory in the local repo to pick one when this is None.
    pub version: Option<String>,
}

/// Parse `<dependency><groupId>...</groupId><artifactId>...</artifactId>` from pom.xml.
///
/// Returns a list of groupId strings (e.g., "org.springframework", "com.google.guava").
/// Lightweight line-based parsing — no XML library needed.
pub fn parse_pom_xml_dependencies(content: &str) -> Vec<String> {
    parse_pom_xml_coords(content)
        .into_iter()
        .map(|c| c.group_id)
        .collect()
}

/// Parse full `<dependency>` coordinates from a pom.xml. Accepts groupId /
/// artifactId / version in any order within a `<dependency>` block.
/// Dependencies that omit `groupId` or `artifactId` are dropped.
/// `<version>` is optional — externals discovery falls back to a
/// version-directory scan when missing.
pub fn parse_pom_xml_coords(content: &str) -> Vec<MavenCoord> {
    let mut coords = Vec::new();
    let mut in_dependency = false;
    let mut gid: Option<String> = None;
    let mut aid: Option<String> = None;
    let mut ver: Option<String> = None;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.contains("<dependency>") {
            in_dependency = true;
            gid = None;
            aid = None;
            ver = None;
            continue;
        }
        if trimmed.contains("</dependency>") {
            if let (Some(g), Some(a)) = (gid.take(), aid.take()) {
                coords.push(MavenCoord {
                    group_id: g,
                    artifact_id: a,
                    version: ver.take(),
                });
            }
            in_dependency = false;
            continue;
        }

        if !in_dependency {
            continue;
        }

        if let Some(value) = extract_xml_text(trimmed, "groupId") {
            gid = Some(value);
        } else if let Some(value) = extract_xml_text(trimmed, "artifactId") {
            aid = Some(value);
        } else if let Some(value) = extract_xml_text(trimmed, "version") {
            // Skip unresolvable property placeholders. The version-dir scan
            // fallback in externals discovery will find a concrete version.
            if !value.starts_with("${") {
                ver = Some(value);
            }
        }
    }

    coords
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
