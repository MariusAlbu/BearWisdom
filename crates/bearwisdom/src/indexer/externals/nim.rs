// Nim / nimble externals

use super::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;
use std::path::{Path, PathBuf};
use tracing::debug;

/// Nim nimble → `discover_nim_externals` + `walk_nim_external_root`.
///
/// Nimble packages are installed to `~/.nimble/pkgs2/<pkg>-<ver>-<hash>/`.
/// Declared deps come from `requires` in `*.nimble` files.
/// Walk: `*.nim` under the package root.
pub struct NimExternalsLocator;

impl ExternalSourceLocator for NimExternalsLocator {
    fn ecosystem(&self) -> &'static str { "nim" }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_nim_externals(project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_nim_external_root(dep)
    }
}

pub fn discover_nim_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    let declared = parse_nimble_requires(project_root);
    if declared.is_empty() {
        return Vec::new();
    }

    let nimble_dir = find_nimble_pkgs_dir();
    let Some(pkgs_dir) = nimble_dir else {
        return Vec::new();
    };

    let mut roots = Vec::new();
    let Ok(entries) = std::fs::read_dir(&pkgs_dir) else { return Vec::new(); };
    let all_entries: Vec<_> = entries.flatten().collect();

    for dep_name in &declared {
        let prefix = format!("{dep_name}-");
        let mut matches: Vec<PathBuf> = all_entries.iter()
            .filter(|e| {
                let n = e.file_name();
                let s = n.to_string_lossy();
                s.starts_with(&prefix) && e.path().is_dir()
            })
            .map(|e| e.path())
            .collect();
        matches.sort();
        if let Some(best) = matches.pop() {
            let version = best.file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_prefix(&prefix))
                .unwrap_or("")
                .to_string();
            roots.push(ExternalDepRoot {
                module_path: dep_name.clone(),
                version,
                root: best,
                ecosystem: "nim",
                package_id: None,
            });
        }
    }
    debug!("Nim: discovered {} external package roots", roots.len());
    roots
}

fn parse_nimble_requires(project_root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(project_root) else {
        return Vec::new();
    };
    let nimble_file = entries.flatten().find(|e| {
        e.path().extension().and_then(|x| x.to_str()) == Some("nimble")
    });
    let Some(entry) = nimble_file else { return Vec::new(); };
    let Ok(content) = std::fs::read_to_string(entry.path()) else { return Vec::new(); };

    let mut deps = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("requires") {
            // requires "nim >= 2.0.0", "jester#hash", "karax#hash"
            for part in trimmed.split('"') {
                let dep = part.trim();
                if dep.is_empty() || dep.starts_with("requires") || dep == "," { continue; }
                // Extract package name (before any version/hash specifier)
                let name = dep.split(|c: char| c == '>' || c == '<' || c == '=' || c == '#' || c == '@' || c.is_whitespace())
                    .next()
                    .unwrap_or("")
                    .trim();
                if !name.is_empty() && name != "nim" && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                    if !deps.contains(&name.to_string()) {
                        deps.push(name.to_string());
                    }
                }
            }
        }
    }
    deps
}

fn find_nimble_pkgs_dir() -> Option<PathBuf> {
    if let Ok(nimble_dir) = std::env::var("NIMBLE_DIR") {
        let p = PathBuf::from(&nimble_dir).join("pkgs2");
        if p.is_dir() { return Some(p); }
        let p = PathBuf::from(nimble_dir).join("pkgs");
        if p.is_dir() { return Some(p); }
    }
    let home = dirs::home_dir()?;
    let pkgs2 = home.join(".nimble").join("pkgs2");
    if pkgs2.is_dir() { return Some(pkgs2); }
    let pkgs = home.join(".nimble").join("pkgs");
    if pkgs.is_dir() { return Some(pkgs); }
    None
}

pub fn walk_nim_external_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_nim_dir(&dep.root, &dep.root, dep, &mut out);
    out
}

fn walk_nim_dir(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>) {
    walk_nim_dir_bounded(dir, root, dep, out, 0);
}

fn walk_nim_dir_bounded(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= MAX_WALK_DEPTH { return; }
    let Ok(entries) = std::fs::read_dir(dir) else { return; };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else { continue; };
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "tests" | "test" | "examples" | "docs" | "nimcache")
                    || name.starts_with('.')
                {
                    continue;
                }
            }
            walk_nim_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue; };
            if !name.ends_with(".nim") { continue; }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            out.push(WalkedFile {
                relative_path: format!("ext:nim:{}/{}", dep.module_path, rel_sub),
                absolute_path: path,
                language: "nim",
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nim_parses_nimble_requires() {
        let tmp = std::env::temp_dir().join("bw-test-nim-parse");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("test.nimble"), r#"
requires "nim >= 2.0.0"
requires "jester#baca3f"
requires "karax#5cf360c"
"#).unwrap();
        let deps = parse_nimble_requires(&tmp);
        assert_eq!(deps, vec!["jester", "karax"]);

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
