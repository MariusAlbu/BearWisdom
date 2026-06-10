// =============================================================================
// ecosystem/manifest/pip_requirements.rs — `requirements*.txt` reader
//
// pip's `requirements.txt` format. Many real Python projects use this
// alongside (or instead of) `pyproject.toml`. Each non-comment line is
// a package spec:
//
//   numpy>=1.20
//   requests==2.28.1
//   black ; python_version >= '3.7'
//   -e git+https://github.com/foo/bar@v1#egg=bar
//   -r dev-requirements.txt
//
// We extract the package name (everything before the first version
// specifier / extras bracket / environment marker / whitespace).
// =============================================================================

use std::path::Path;

use super::{ManifestData, ManifestKind, ManifestReader, ReaderEntry};

pub struct PipRequirementsManifest;

impl ManifestReader for PipRequirementsManifest {
    fn kind(&self) -> ManifestKind {
        ManifestKind::PipRequirements
    }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let mut data = ManifestData::default();
        let mut found_any = false;
        for name in candidate_filenames() {
            let path = project_root.join(name);
            if let Ok(content) = std::fs::read_to_string(&path) {
                found_any = true;
                for pkg in parse_requirements(&content) {
                    data.dependencies.insert(pkg);
                }
            }
        }
        if found_any {
            Some(data)
        } else {
            None
        }
    }

    fn read_all(&self, project_root: &Path) -> Vec<ReaderEntry> {
        // pip's requirements.txt is typically at the project root, occasionally
        // in `requirements/` subdirs. Don't walk the whole tree — keep cost
        // bounded.
        let mut out = Vec::new();
        if let Some(data) = self.read(project_root) {
            out.push(ReaderEntry {
                package_dir: project_root.to_path_buf(),
                manifest_path: project_root.join("requirements.txt"),
                data,
                name: None,
            });
        }
        // Common requirements/ subdirectory.
        let req_dir = project_root.join("requirements");
        if req_dir.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&req_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                        continue;
                    };
                    if !name.ends_with(".txt") {
                        continue;
                    }
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        let mut data = ManifestData::default();
                        for pkg in parse_requirements(&content) {
                            data.dependencies.insert(pkg);
                        }
                        out.push(ReaderEntry {
                            package_dir: req_dir.clone(),
                            manifest_path: path,
                            data,
                            name: None,
                        });
                    }
                }
            }
        }
        out
    }
}

fn candidate_filenames() -> &'static [&'static str] {
    &[
        "requirements.txt",
        "requirements-dev.txt",
        "requirements-test.txt",
        "requirements_dev.txt",
        "requirements_test.txt",
        "dev-requirements.txt",
        "test-requirements.txt",
    ]
}

/// Extract package names from a pip-style requirements file.
pub fn parse_requirements(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        // Skip pip options that don't introduce a package: -r, -c, -e (without egg)
        if trimmed.starts_with("--") || trimmed.starts_with("-r ") || trimmed.starts_with("-c ") {
            continue;
        }
        // `-e <url>#egg=name` — extract name from egg fragment.
        if trimmed.starts_with("-e ") {
            if let Some(idx) = trimmed.find("#egg=") {
                let rest = &trimmed[idx + 5..];
                let name = rest
                    .split(|c: char| c == '&' || c.is_whitespace())
                    .next()
                    .unwrap_or("");
                if !name.is_empty() {
                    out.push(normalise_pkg(name))
                }
            }
            continue;
        }
        // Strip environment marker (`pkg ; python_version >= '3.7'`).
        let before_marker = trimmed.split(';').next().unwrap_or(trimmed).trim();
        // Strip extras bracket (`pkg[extra1,extra2]`).
        let before_bracket = before_marker
            .split('[')
            .next()
            .unwrap_or(before_marker)
            .trim();
        // Strip version specifiers (`pkg>=1.2`, `pkg==1.0`, `pkg~=1`, `pkg<2`).
        let name_end = before_bracket
            .find(|c: char| matches!(c, '=' | '<' | '>' | '!' | '~' | ' '))
            .unwrap_or(before_bracket.len());
        let name = &before_bracket[..name_end];
        if name.is_empty() {
            continue;
        }
        out.push(normalise_pkg(name));
    }
    out
}

fn normalise_pkg(name: &str) -> String {
    // PEP 503: lowercase, replace runs of `_-.` with single `-`.
    name.to_lowercase()
        .chars()
        .map(|c| if matches!(c, '_' | '.') { '-' } else { c })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
#[path = "pip_requirements_tests.rs"]
mod tests;
