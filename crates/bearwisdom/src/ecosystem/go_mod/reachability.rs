// ---------------------------------------------------------------------------
// Reachability: narrow to user-requested sub-packages + transitive within-module
// ---------------------------------------------------------------------------
//
// Go is unlike npm/pypi/cargo because a single module exposes many
// independent sub-packages (flat-directory `package X { ... }` files, no
// explicit re-export mechanism). A typical user project imports only a
// handful of sub-packages from each module. The eager walk_root strategy
// indexes every package in the module even when the user's surface is
// narrow — wasteful on monster modules like `k8s.io/api` or
// `google.golang.org/protobuf`.
//
// Reachability for Go: walk only the sub-directories corresponding to the
// user's import paths (stored on the dep root at discovery time) plus any
// within-module imports those packages pull in transitively. Each Go
// package is flat (no recursion into subdirs — subdirs are separate
// packages with their own imports), so the walk per package is a single
// `read_dir` scan.

use std::path::{Path, PathBuf};

use super::discovery::extract_imports_from_go_source;
use super::walk_go_root;
use crate::ecosystem::externals::ExternalDepRoot;
use crate::walker::WalkedFile;

const GO_SUBPKG_MAX_DEPTH: u32 = 3;

pub(super) fn resolve_go_requested_packages(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    if dep.requested_imports.is_empty() {
        // Discovery didn't populate demand data; fall back to the eager walk
        // so behavior matches pre-R3 when user_imports scanning was absent.
        return walk_go_root(dep);
    }

    let mut out = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for import_path in &dep.requested_imports {
        let Some(sub) = import_path.strip_prefix(&dep.module_path[..]) else {
            continue;
        };
        let sub = sub.trim_start_matches('/');
        let pkg_dir = if sub.is_empty() {
            dep.root.clone()
        } else {
            dep.root
                .join(sub.replace('/', std::path::MAIN_SEPARATOR_STR.to_string().as_str()))
        };
        expand_go_package_into(dep, &pkg_dir, &mut out, &mut seen, 0);
    }
    out
}

fn expand_go_package_into(
    dep: &ExternalDepRoot,
    pkg_dir: &Path,
    out: &mut Vec<WalkedFile>,
    seen: &mut std::collections::HashSet<PathBuf>,
    depth: u32,
) {
    if !pkg_dir.is_dir() {
        return;
    }
    if !seen.insert(pkg_dir.to_path_buf()) {
        return;
    }

    let Ok(entries) = std::fs::read_dir(pkg_dir) else {
        return;
    };
    let mut package_sources: Vec<std::path::PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_file() {
            continue;
        }
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.ends_with(".go") {
            continue;
        }
        if name.ends_with("_test.go") {
            continue;
        }
        if !crate::ecosystem::go_platform::file_matches_host(name) {
            continue;
        }
        let rel_sub = match path.strip_prefix(&dep.root) {
            Ok(p) => p.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };
        out.push(WalkedFile {
            relative_path: format!("ext:{}@{}/{}", dep.module_path, dep.version, rel_sub),
            absolute_path: path.clone(),
            language: "go",
        });
        package_sources.push(path);
    }

    if depth >= GO_SUBPKG_MAX_DEPTH {
        return;
    }

    // Scan this package's source for within-module imports and pull those
    // sub-packages in too. Without this, a user-imported package that
    // re-exports types from a sibling sub-package loses the type
    // definitions it needs.
    let module_prefix = format!("{}/", dep.module_path);
    for path in &package_sources {
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        let mut imports: std::collections::HashSet<String> = std::collections::HashSet::new();
        extract_imports_from_go_source(&content, &mut imports);
        for imp in imports {
            if !imp.starts_with(&module_prefix) {
                continue;
            }
            let Some(sub) = imp.strip_prefix(&dep.module_path[..]) else {
                continue;
            };
            let sub = sub.trim_start_matches('/');
            if sub.is_empty() {
                continue;
            }
            let sub_dir = dep
                .root
                .join(sub.replace('/', std::path::MAIN_SEPARATOR_STR.to_string().as_str()));
            expand_go_package_into(dep, &sub_dir, out, seen, depth + 1);
        }
    }
}
