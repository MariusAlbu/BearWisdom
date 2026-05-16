// ===========================================================================
// Manifest reader (migrated from indexer/manifest/pyproject.rs)
// ===========================================================================

use std::path::{Path, PathBuf};

use crate::ecosystem::manifest::{ManifestData, ManifestKind, ManifestReader};

pub struct PyProjectManifest;

impl ManifestReader for PyProjectManifest {
    fn kind(&self) -> ManifestKind { ManifestKind::PyProject }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let mut manifest_files: Vec<(PathBuf, &str)> = Vec::new();
        collect_python_manifests(project_root, &mut manifest_files, 0);
        if manifest_files.is_empty() { return None }

        let mut data = ManifestData::default();
        for (path, kind) in &manifest_files {
            let content = match std::fs::read_to_string(path) { Ok(c) => c, Err(_) => continue };
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

fn collect_python_manifests<'a>(
    dir: &Path,
    out: &mut Vec<(PathBuf, &'a str)>,
    depth: usize,
) {
    if depth > 6 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if matches!(
                name.as_ref(),
                ".git" | "node_modules" | "target" | "__pycache__"
                    | ".venv" | "venv" | ".tox" | "dist" | "build" | ".eggs"
            ) { continue }
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
            } else { continue };
            out.push((path, kind));
        }
    }
}

/// Parse package names from `pyproject.toml` covering:
///   * PEP 621 `[project]` dependencies array + `[project.optional-dependencies.*]`
///   * PEP 735 `[dependency-groups]` (the modern dev-deps format that uv,
///     hatch, and PDM read; paperless-ngx, Django itself, and most
///     pyproject.toml-driven projects in 2026 use it)
///   * Poetry `[tool.poetry.dependencies]` + dev-dependencies / group.X
///   * Poetry `[tool.poetry.group.<name>.dependencies]`
///
/// Without `[dependency-groups]` and `[project.optional-dependencies]`
/// support, dev/test deps (pytest, factory-boy, pytest-django, …) stay
/// invisible to the externals walker and every test-framework call goes
/// unresolved — hits hardest on Django projects whose test suites are
/// the dominant ref source.
pub fn parse_pyproject_deps(content: &str) -> Vec<String> {
    let mut packages = Vec::new();
    let mut in_deps = false;
    let mut in_array = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            // Headers that put us in a "every line names a dep" section.
            // Cover all of: `[dependency-groups]` (PEP 735),
            // `[project.optional-dependencies]` (PEP 621), Poetry's
            // dependencies / dev-dependencies / `group.<name>.dependencies`,
            // plus the parent `[project]` table whose `dependencies = [...]`
            // array we capture via the `in_array` path below.
            in_deps = trimmed == "[project.dependencies]"
                || trimmed == "[tool.poetry.dependencies]"
                || trimmed == "[tool.poetry.dev-dependencies]"
                || trimmed.starts_with("[tool.poetry.group.")
                || trimmed == "[dependency-groups]"
                || trimmed.starts_with("[project.optional-dependencies")
                || trimmed.starts_with("[tool.poetry.extras")
                || trimmed == "[project]";
            in_array = false;
            continue;
        }
        if trimmed.starts_with("dependencies") && trimmed.contains('=') {
            let rest = trimmed.splitn(2, '=').nth(1).unwrap_or("").trim();
            in_array = rest.starts_with('[') && !rest.contains(']');
            let data = if rest.starts_with('[') {
                let inner = rest.trim_start_matches('[');
                inner.trim_end_matches(']')
            } else { rest };
            for name in extract_pep508_names(data) { packages.push(name) }
            if rest.contains(']') { in_array = false }
            continue;
        }
        if in_array {
            if trimmed.starts_with(']') { in_array = false }
            for name in extract_pep508_names(trimmed) { packages.push(name) }
            continue;
        }
        if in_deps && !trimmed.starts_with('[') && trimmed.contains('=') {
            // PEP 735 / `[project.optional-dependencies]` shape: a key
            // names a group whose value is an array of PEP 508 specs.
            //   testing = [
            //     "pytest~=9.0.0",
            //     "factory-boy~=3.3.1",
            //   ]
            // Poetry's `[tool.poetry.dependencies]` shape: each line is
            // a single dep, key = name, value = version constraint.
            //   pytest = "^9.0"
            // Distinguish by whether the value is a `[`-prefixed array
            // (PEP 735) or a scalar (Poetry).
            let (key_part, value_part) = trimmed.split_once('=').unwrap_or((trimmed, ""));
            let key = key_part.trim();
            let value = value_part.trim();
            if !key.is_empty()
                && key != "python"
                && key.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
            {
                if value.starts_with('[') {
                    // PEP 735 / optional-dependencies array. Same
                    // dependencies-array machinery as above — the
                    // following lines hold PEP 508 strings until `]`.
                    in_array = !value.contains(']');
                    let inner = value
                        .trim_start_matches('[')
                        .trim_end_matches(']');
                    for name in extract_pep508_names(inner) { packages.push(name) }
                } else {
                    packages.push(key.to_string());
                }
            }
        }
    }
    packages
}

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

pub fn parse_requirements_txt(content: &str) -> Vec<String> {
    let mut packages = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with('#')
            || trimmed.starts_with('-')
            || trimmed.starts_with("git+")
            || trimmed.starts_with("http")
        { continue }
        let without_comment = trimmed.split('#').next().unwrap_or(trimmed).trim();
        let end = without_comment
            .find(|c: char| matches!(c, '[' | '>' | '<' | '=' | '!' | ';' | '@' | ' '))
            .unwrap_or(without_comment.len());
        let name = without_comment[..end].trim();
        if !name.is_empty() { packages.push(name.to_string()) }
    }
    packages
}

pub fn parse_pipfile_deps(content: &str) -> Vec<String> {
    let mut packages = Vec::new();
    let mut in_section = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_section = matches!(trimmed, "[packages]" | "[dev-packages]");
            continue;
        }
        if !in_section || trimmed.is_empty() || trimmed.starts_with('#') { continue }
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
