// ---------------------------------------------------------------------------
// Reachability: package entry + bounded relative-import expansion
// ---------------------------------------------------------------------------

use std::path::{Path, PathBuf};

use crate::ecosystem::externals::ExternalDepRoot;
use crate::walker::WalkedFile;

const PY_REEXPORT_MAX_DEPTH: u32 = 3;

/// Start from the package's entry (`__init__.py` for directory packages, the
/// .py file itself for single-file modules) and follow relative imports
/// bounded at depth 3. Most packages surface their public API through
/// `__init__.py` with `from .sub import X` re-exports; this walks that
/// surface without indexing unrelated internal modules.
pub(super) fn resolve_python_package_entry(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

    if dep.root.is_file() {
        // Single-file module (e.g., `six.py` in site-packages).
        expand_python_reexports_into(dep, &dep.root, &dep.root, &mut out, &mut seen, 0);
        return out;
    }

    let init = dep.root.join("__init__.py");
    if init.is_file() {
        expand_python_reexports_into(dep, &dep.root, &init, &mut out, &mut seen, 0);
    }
    out
}

pub(super) fn expand_python_reexports_into(
    dep: &ExternalDepRoot,
    pkg_root: &Path,
    file: &Path,
    out: &mut Vec<WalkedFile>,
    seen: &mut std::collections::HashSet<PathBuf>,
    depth: u32,
) {
    if !seen.insert(file.to_path_buf()) { return }
    if !file.is_file() { return }

    let rel_sub = match file.strip_prefix(pkg_root) {
        Ok(p) => p.to_string_lossy().replace('\\', "/"),
        Err(_) => file
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("module.py")
            .to_string(),
    };
    out.push(WalkedFile {
        relative_path: format!("ext:py:{}/{}", dep.module_path, rel_sub),
        absolute_path: file.to_path_buf(),
        language: "python",
    });

    if depth >= PY_REEXPORT_MAX_DEPTH { return }

    let Ok(src) = std::fs::read_to_string(file) else { return };
    for target in extract_python_relative_imports(&src) {
        let Some(next) = resolve_python_relative_path(file, pkg_root, &target) else {
            continue;
        };
        expand_python_reexports_into(dep, pkg_root, &next, out, seen, depth + 1);
    }
}

/// Scan for `from .sub import X`, `from .sub.deep import *`, `from . import X`
/// patterns. Returns `(relative_module_spec, ...)` in source order. Bare
/// imports (`from foo import X`) are skipped — those are separate packages
/// with their own dep roots.
fn extract_python_relative_imports(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in src.lines() {
        let t = line.trim_start();
        if !t.starts_with("from ") { continue }
        // Take the portion between `from ` and ` import`
        let after_from = &t[5..];
        let Some(import_ix) = after_from.find(" import ") else { continue };
        let spec = after_from[..import_ix].trim();
        // Relative specs start with one or more dots.
        if !spec.starts_with('.') { continue }
        out.push(spec.to_string());
    }
    out
}

/// Resolve a relative-import spec against the current file within the
/// package root. Handles `.sub`, `..parent_sibling`, `.sub.deep`, bare `.`.
/// Returns either a `.py` file or the `__init__.py` of the target directory.
fn resolve_python_relative_path(from_file: &Path, pkg_root: &Path, spec: &str) -> Option<PathBuf> {
    // Count leading dots.
    let mut dots = 0usize;
    for c in spec.chars() {
        if c == '.' { dots += 1 } else { break }
    }
    if dots == 0 { return None }
    let rest = &spec[dots..]; // may be empty

    // Walk up `dots - 1` levels from the file's parent directory.
    let mut base = from_file.parent()?.to_path_buf();
    for _ in 1..dots {
        base = base.parent()?.to_path_buf();
    }

    // Safety: don't escape the package root.
    if let Ok(canon_pkg) = pkg_root.canonicalize() {
        if let Ok(canon_base) = base.canonicalize() {
            if !canon_base.starts_with(&canon_pkg) { return None }
        }
    }

    // Build the target path by joining dotted segments.
    let mut target = base;
    for seg in rest.split('.').filter(|s| !s.is_empty()) {
        target = target.join(seg);
    }

    // Try as file first: target + .py
    let as_file = target.with_extension("py");
    if as_file.is_file() { return Some(as_file) }
    // Fallback: target is a directory with __init__.py
    let as_init = target.join("__init__.py");
    if as_init.is_file() { return Some(as_init) }
    None
}
