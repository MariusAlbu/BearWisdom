// ===========================================================================
// Manifest reader (migrated from indexer/manifest/go_mod.rs)
// ===========================================================================

use std::path::{Path, PathBuf};

use crate::ecosystem::manifest::{ManifestData, ManifestKind, ManifestReader};

pub struct GoModManifest;

impl ManifestReader for GoModManifest {
    fn kind(&self) -> ManifestKind {
        ManifestKind::GoMod
    }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let go_mod_path = find_go_mod(project_root)?;
        let content = std::fs::read_to_string(&go_mod_path).ok()?;
        let parsed = parse_go_mod(&content);
        let mut data = ManifestData::default();
        data.module_path = parsed.module_path;
        for path in parsed.require_paths {
            data.dependencies.insert(path);
        }
        Some(data)
    }
}

pub struct GoModData {
    pub module_path: Option<String>,
    pub require_paths: Vec<String>,
    pub require_deps: Vec<GoModDep>,
}

#[derive(Debug, Clone)]
pub struct GoModDep {
    pub path: String,
    pub version: String,
    pub indirect: bool,
}

pub fn find_go_mod(root: &Path) -> Option<PathBuf> {
    let candidate = root.join("go.mod");
    if candidate.is_file() {
        return Some(candidate);
    }
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                let nested = entry.path().join("go.mod");
                if nested.is_file() {
                    return Some(nested);
                }
            }
        }
    }
    None
}

pub fn parse_go_mod(content: &str) -> GoModData {
    let mut module_path: Option<String> = None;
    let mut require_paths = Vec::new();
    let mut require_deps = Vec::new();
    let mut in_require_block = false;

    fn parse_dep(fragment: &str) -> Option<GoModDep> {
        let without_comment = fragment.trim();
        let (main, comment) = match without_comment.find("//") {
            Some(idx) => (without_comment[..idx].trim(), &without_comment[idx..]),
            None => (without_comment, ""),
        };
        let mut tokens = main.split_whitespace();
        let path = tokens.next()?.to_string();
        let version = tokens.next()?.to_string();
        let indirect = comment.contains("indirect");
        Some(GoModDep {
            path,
            version,
            indirect,
        })
    }

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("module ") {
            let path = rest.split_whitespace().next().unwrap_or("").trim();
            if !path.is_empty() {
                module_path = Some(path.to_string())
            }
            continue;
        }
        if trimmed == "require (" || trimmed.starts_with("require (") {
            in_require_block = true;
            continue;
        }
        if trimmed == ")" {
            in_require_block = false;
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("require ") {
            let rest = rest.trim();
            if rest != "(" && !rest.is_empty() {
                if let Some(dep) = parse_dep(rest) {
                    require_paths.push(dep.path.clone());
                    require_deps.push(dep);
                }
            }
            continue;
        }
        if in_require_block && !trimmed.starts_with("//") {
            if let Some(dep) = parse_dep(trimmed) {
                require_paths.push(dep.path.clone());
                require_deps.push(dep);
            }
        }
    }

    GoModData {
        module_path,
        require_paths,
        require_deps,
    }
}
