// ===========================================================================
// Discovery — $GOMODCACHE / GOPATH / ~/go/pkg/mod
// ===========================================================================

use std::path::{Path, PathBuf};

use tracing::debug;

use super::manifest::{find_go_mod, parse_go_mod, GoModDep};
use super::LEGACY_ECOSYSTEM_TAG;
use crate::ecosystem::externals::ExternalDepRoot;

pub fn discover_go_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    let Some(go_mod_path) = find_go_mod(project_root) else { return Vec::new() };
    let Ok(content) = std::fs::read_to_string(&go_mod_path) else { return Vec::new() };
    let parsed = parse_go_mod(&content);

    let cache_root = match gomodcache_root() {
        Some(p) => p,
        None => {
            debug!("No GOMODCACHE / GOPATH detected; skipping Go externals");
            return Vec::new();
        }
    };

    let user_imports = collect_go_imports(project_root);

    let mut roots = Vec::new();
    for dep in &parsed.require_deps {
        if dep.indirect && !go_dep_is_imported(&dep.path, &user_imports) { continue }
        if let Some(root) = resolve_go_dep_path(&cache_root, dep) {
            let requested = collect_module_imports(&dep.path, &user_imports);
            roots.push(ExternalDepRoot {
                module_path: dep.path.clone(),
                version: dep.version.clone(),
                root,
                ecosystem: LEGACY_ECOSYSTEM_TAG,
                package_id: None,
                requested_imports: requested,
            });
        } else {
            debug!(
                "Go module cache miss for {}@{} — not found under {}",
                dep.path,
                dep.version,
                cache_root.display()
            );
        }
    }
    roots
}

/// Return the list of user import paths that live under `module_path`.
/// Includes the module root import itself (the main package) plus every
/// sub-package the project references. Paths are stored verbatim so
/// `resolve_go_requested_packages` can strip the module prefix when
/// mapping to on-disk subdirs.
pub(super) fn collect_module_imports(
    module_path: &str,
    user_imports: &std::collections::HashSet<String>,
) -> Vec<String> {
    let mut out = Vec::new();
    let prefix = format!("{module_path}/");
    if user_imports.contains(module_path) { out.push(module_path.to_string()) }
    for imp in user_imports {
        if imp.starts_with(&prefix) { out.push(imp.clone()) }
    }
    out.sort();
    out.dedup();
    out
}

fn collect_go_imports(project_root: &Path) -> std::collections::HashSet<String> {
    let mut imports: std::collections::HashSet<String> = std::collections::HashSet::new();
    scan_go_imports_recursive(project_root, &mut imports, 0);
    imports
}

fn scan_go_imports_recursive(
    dir: &Path,
    out: &mut std::collections::HashSet<String>,
    depth: usize,
) {
    if depth > 10 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if let Ok(ft) = entry.file_type() {
            if ft.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if matches!(
                        name,
                        ".git" | "vendor" | "node_modules" | "target"
                            | "build" | "dist" | "testdata"
                    ) { continue }
                }
                scan_go_imports_recursive(&path, out, depth + 1);
            } else if ft.is_file() {
                let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
                if !name.ends_with(".go") || name.ends_with("_test.go") { continue }
                let Ok(content) = std::fs::read_to_string(&path) else { continue };
                extract_imports_from_go_source(&content, out);
            }
        }
    }
}

pub(super) fn extract_imports_from_go_source(content: &str, out: &mut std::collections::HashSet<String>) {
    enum Mode { Top, InBlock }
    let mut mode = Mode::Top;
    for line in content.lines() {
        let trimmed = line.trim();
        match mode {
            Mode::Top => {
                if trimmed.starts_with("import (") { mode = Mode::InBlock; continue }
                if let Some(rest) = trimmed.strip_prefix("import ") {
                    let rest = rest.trim_start_matches('_').trim();
                    let quoted = rest
                        .rsplit_once('"')
                        .map(|(head, _)| head)
                        .and_then(|head| head.rsplit_once('"').map(|(_, s)| s));
                    if let Some(path) = quoted {
                        if !path.is_empty() { out.insert(path.to_string()); }
                    }
                }
            }
            Mode::InBlock => {
                if trimmed == ")" { mode = Mode::Top; continue }
                let bytes = trimmed.as_bytes();
                let first = bytes.iter().position(|&b| b == b'"');
                let Some(start) = first else { continue };
                let after = &trimmed[start + 1..];
                let Some(end_rel) = after.find('"') else { continue };
                let path = &after[..end_rel];
                if !path.is_empty() { out.insert(path.to_string()); }
            }
        }
    }
}

fn go_dep_is_imported(
    dep_path: &str,
    user_imports: &std::collections::HashSet<String>,
) -> bool {
    if user_imports.contains(dep_path) { return true }
    let prefix = format!("{dep_path}/");
    user_imports.iter().any(|imp| imp.starts_with(&prefix))
}

pub fn gomodcache_root() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("GOMODCACHE") {
        let p = PathBuf::from(explicit);
        if p.is_dir() { return Some(p) }
    }
    if let Some(gopath) = std::env::var_os("GOPATH") {
        let first = PathBuf::from(gopath)
            .to_string_lossy()
            .split(|c| c == ':' || c == ';')
            .next()
            .map(PathBuf::from);
        if let Some(p) = first {
            let candidate = p.join("pkg").join("mod");
            if candidate.is_dir() { return Some(candidate) }
        }
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let candidate = PathBuf::from(home).join("go").join("pkg").join("mod");
    if candidate.is_dir() { Some(candidate) } else { None }
}

fn resolve_go_dep_path(cache_root: &Path, dep: &GoModDep) -> Option<PathBuf> {
    let escaped = escape_module_path(&dep.path);
    let dirname = format!("{}@{}", escaped, dep.version);
    let candidate = cache_root.join(dirname.replace('/', std::path::MAIN_SEPARATOR_STR));
    if candidate.is_dir() { return Some(candidate) }
    let mut segments: Vec<&str> = escaped.split('/').collect();
    let last = segments.pop()?;
    let mut path = cache_root.to_path_buf();
    for seg in segments { path.push(seg); }
    path.push(format!("{last}@{}", dep.version));
    if path.is_dir() { Some(path) } else { None }
}

pub(super) fn escape_module_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len() + 4);
    for ch in path.chars() {
        if ch.is_ascii_uppercase() {
            out.push('!');
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}
