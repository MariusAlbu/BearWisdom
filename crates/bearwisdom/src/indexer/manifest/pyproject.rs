// indexer/manifest/pyproject.rs — Python manifest reader
// Handles: pyproject.toml, requirements.txt, Pipfile

use std::path::Path;

use super::{ManifestData, ManifestKind, ManifestReader};

pub struct PyProjectManifest;

impl ManifestReader for PyProjectManifest {
    fn kind(&self) -> ManifestKind {
        ManifestKind::PyProject
    }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let mut manifest_files: Vec<(std::path::PathBuf, &str)> = Vec::new();
        collect_python_manifests(project_root, &mut manifest_files, 0);

        if manifest_files.is_empty() {
            return None;
        }

        let mut data = ManifestData::default();

        for (path, kind) in &manifest_files {
            let content = match std::fs::read_to_string(path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let names = match *kind {
                "pyproject" => parse_pyproject_deps(&content),
                "requirements" => parse_requirements_txt(&content),
                "pipfile" => parse_pipfile_deps(&content),
                _ => Vec::new(),
            };
            for name in names {
                data.dependencies.insert(name);
            }
        }

        Some(data)
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn collect_python_manifests<'a>(
    dir: &Path,
    out: &mut Vec<(std::path::PathBuf, &'a str)>,
    depth: usize,
) {
    if depth > 6 {
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
                ".git"
                    | "node_modules"
                    | "target"
                    | "__pycache__"
                    | ".venv"
                    | "venv"
                    | ".tox"
                    | "dist"
                    | "build"
                    | ".eggs"
            ) {
                continue;
            }
            collect_python_manifests(&path, out, depth + 1);
        } else {
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();
            let kind: &'a str = if name == "pyproject.toml" {
                "pyproject"
            } else if name == "requirements.txt"
                || (name.starts_with("requirements") && name.ends_with(".txt"))
            {
                "requirements"
            } else if name == "Pipfile" {
                "pipfile"
            } else {
                continue;
            };
            out.push((path, kind));
        }
    }
}

/// Parse package names from `pyproject.toml`.
///
/// Handles both PEP 621 (`[project] dependencies`) and Poetry
/// (`[tool.poetry.dependencies]`) formats. Line-based; no full TOML parser.
pub fn parse_pyproject_deps(content: &str) -> Vec<String> {
    let mut packages = Vec::new();
    let mut in_deps = false;
    let mut in_array = false;

    for line in content.lines() {
        let trimmed = line.trim();

        // Section detection.
        if trimmed.starts_with('[') {
            in_deps = matches!(
                trimmed,
                "[project.dependencies]"
                    | "[tool.poetry.dependencies]"
                    | "[tool.poetry.dev-dependencies]"
                    | "[tool.poetry.group.dev.dependencies]"
            ) || trimmed == "[project]";

            // Reset inline array state on any section boundary.
            in_array = false;
            continue;
        }

        // PEP 621: `dependencies = ["django>=4.0", "pydantic"]` (may span lines)
        if trimmed.starts_with("dependencies") && trimmed.contains('=') {
            let rest = trimmed.splitn(2, '=').nth(1).unwrap_or("").trim();
            in_array = rest.starts_with('[') && !rest.contains(']');

            let data = if rest.starts_with('[') {
                let inner = rest.trim_start_matches('[');
                let inner = inner.trim_end_matches(']');
                inner
            } else {
                rest
            };
            for name in extract_pep508_names(data) {
                packages.push(name);
            }
            if rest.contains(']') {
                in_array = false;
            }
            continue;
        }

        if in_array {
            // Only end the array on a standalone `]` — not on a dependency line
            // that contains extras like `"celery[redis]~=5.6.2"`.
            if trimmed.starts_with(']') {
                in_array = false;
            }
            for name in extract_pep508_names(trimmed) {
                packages.push(name);
            }
            continue;
        }

        // Poetry format: `django = "^4.0"` inside a deps section.
        if in_deps && !trimmed.starts_with('[') && trimmed.contains('=') {
            let key = trimmed.split('=').next().unwrap_or("").trim();
            if !key.is_empty()
                && key != "python"
                && key.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
            {
                packages.push(key.to_string());
            }
        }
    }

    packages
}

/// Parse a PEP 508 dependency specifier list and extract package names.
fn extract_pep508_names(s: &str) -> Vec<String> {
    let mut names = Vec::new();
    for part in s.split(',') {
        let part = part.trim().trim_matches(|c| c == '"' || c == '\'' || c == ']');
        let end = part
            .find(|c: char| matches!(c, '[' | '>' | '<' | '=' | '~' | '!' | ';' | '@' | ' '))
            .unwrap_or(part.len());
        let name = part[..end].trim();
        if !name.is_empty()
            && name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
        {
            names.push(name.to_string());
        }
    }
    names
}

/// Parse package names from a `requirements.txt` file.
pub fn parse_requirements_txt(content: &str) -> Vec<String> {
    let mut packages = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with('#')
            || trimmed.starts_with('-')
            || trimmed.starts_with("git+")
            || trimmed.starts_with("http")
        {
            continue;
        }
        let without_comment = trimmed.split('#').next().unwrap_or(trimmed).trim();
        let end = without_comment
            .find(|c: char| matches!(c, '[' | '>' | '<' | '=' | '!' | ';' | '@' | ' '))
            .unwrap_or(without_comment.len());
        let name = without_comment[..end].trim();
        if !name.is_empty() {
            packages.push(name.to_string());
        }
    }
    packages
}

/// Parse package names from a `Pipfile`.
pub fn parse_pipfile_deps(content: &str) -> Vec<String> {
    let mut packages = Vec::new();
    let mut in_section = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with('[') {
            in_section = matches!(trimmed, "[packages]" | "[dev-packages]");
            continue;
        }

        if !in_section || trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if let Some(eq_pos) = trimmed.find('=') {
            let key = trimmed[..eq_pos].trim();
            if !key.is_empty()
                && key.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
            {
                packages.push(key.to_string());
            }
        }
    }

    packages
}
