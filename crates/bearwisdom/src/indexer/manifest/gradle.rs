// indexer/manifest/gradle.rs — build.gradle / build.gradle.kts reader

use std::path::Path;

use super::{ManifestData, ManifestKind, ManifestReader};

pub struct GradleManifest;

impl ManifestReader for GradleManifest {
    fn kind(&self) -> ManifestKind {
        ManifestKind::Gradle
    }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let mut gradle_paths = Vec::new();
        collect_gradle_files(project_root, &mut gradle_paths, 0);

        if gradle_paths.is_empty() {
            return None;
        }

        let mut data = ManifestData::default();
        for path in &gradle_paths {
            let content = match std::fs::read_to_string(path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            for group_id in parse_gradle_dependencies(&content) {
                data.dependencies.insert(group_id);
            }
        }
        Some(data)
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn collect_gradle_files(dir: &Path, out: &mut Vec<std::path::PathBuf>, depth: usize) {
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
            collect_gradle_files(&path, out, depth + 1);
        } else {
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();
            if name == "build.gradle" || name == "build.gradle.kts" {
                out.push(path);
            }
        }
    }
}

/// Parse dependency declarations from build.gradle / build.gradle.kts.
///
/// Handles the common forms:
///   `implementation 'group:artifact:version'`
///   `implementation("group:artifact:version")`
///   `testImplementation 'group:artifact:version'`
///   `api 'group:artifact:version'`
///
/// Returns a list of groupId strings extracted from the coordinates.
pub fn parse_gradle_dependencies(content: &str) -> Vec<String> {
    let mut group_ids = Vec::new();

    let dependency_keywords = [
        "implementation",
        "testImplementation",
        "api",
        "compileOnly",
        "runtimeOnly",
        "testCompileOnly",
        "annotationProcessor",
        "kapt",
    ];

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with('#') {
            continue;
        }

        let mut found_keyword = false;
        let mut rest = trimmed;
        for kw in &dependency_keywords {
            if let Some(r) = trimmed.strip_prefix(kw) {
                let r = r.trim();
                if r.is_empty()
                    || r.starts_with(' ')
                    || r.starts_with('(')
                    || r.starts_with('"')
                    || r.starts_with('\'')
                {
                    rest = r.trim_start_matches(['(', ' ']);
                    found_keyword = true;
                    break;
                }
            }
        }
        if !found_keyword {
            continue;
        }

        let coord = if let Some(r) = rest.strip_prefix('\'') {
            r.split('\'').next().unwrap_or("").trim()
        } else if let Some(r) = rest.strip_prefix('"') {
            r.split('"').next().unwrap_or("").trim()
        } else {
            continue;
        };

        if let Some(group_id) = coord.split(':').next() {
            let group_id = group_id.trim();
            if !group_id.is_empty()
                && group_id
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '.' || c == '-' || c == '_')
            {
                group_ids.push(group_id.to_string());
            }
        }
    }

    group_ids
}
