// ---------------------------------------------------------------------------
// Reachability: package entry + bounded export expansion
// ---------------------------------------------------------------------------
//
// dep.root is already the package's `lib/` directory (set at discovery time).
// The main entry is conventionally `lib/<package>.dart`. It commonly uses
// `export 'src/foo.dart';` to re-publish implementation files under
// `lib/src/` — the current eager walk skips `src/` entirely, which under-
// covers types that are publicly exported from impl files. The reachability
// path fixes that by starting at the entry and following `export '...';`
// statements, while still skipping anything not re-exported.

use std::path::{Path, PathBuf};

use crate::ecosystem::externals::ExternalDepRoot;
use crate::walker::WalkedFile;

const DART_EXPORT_MAX_DEPTH: u32 = 3;

pub(super) fn resolve_dart_package_entry(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let entry = dep.root.join(format!("{}.dart", dep.module_path));
    if !entry.is_file() {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    expand_dart_exports_into(dep, &entry, &mut out, &mut seen, 0);
    out
}

fn expand_dart_exports_into(
    dep: &ExternalDepRoot,
    file: &Path,
    out: &mut Vec<WalkedFile>,
    seen: &mut std::collections::HashSet<PathBuf>,
    depth: u32,
) {
    if !seen.insert(file.to_path_buf()) {
        return;
    }
    if !file.is_file() {
        return;
    }

    let rel_sub = match file.strip_prefix(&dep.root) {
        Ok(p) => p.to_string_lossy().replace('\\', "/"),
        Err(_) => return,
    };
    out.push(WalkedFile {
        relative_path: format!("ext:dart:{}/{}", dep.module_path, rel_sub),
        absolute_path: file.to_path_buf(),
        language: "dart",
    });

    if depth >= DART_EXPORT_MAX_DEPTH {
        return;
    }

    let Ok(src) = std::fs::read_to_string(file) else {
        return;
    };
    for spec in extract_dart_exports(&src, &dep.module_path) {
        let Some(next) = resolve_dart_relative_path(file, &dep.root, &spec) else {
            continue;
        };
        expand_dart_exports_into(dep, &next, out, seen, depth + 1);
    }
}

/// Scan line-oriented for `export '...';` and `export "...";`. Also handles
/// `part '...';` (a file split across multiple files) and
/// `import '...';` with package-internal specifiers. Returns the path-like
/// inner string for relative (`src/foo.dart`) and in-package (`package:
/// <this_pkg>/foo.dart`) specifiers; external `package:other/...` imports
/// are skipped — they resolve via their own dep root.
pub(super) fn extract_dart_exports(src: &str, this_pkg: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in src.lines() {
        let line = raw.trim_start();
        let kind = if line.starts_with("export ") {
            &line[7..]
        } else if line.starts_with("part ") {
            &line[5..]
        } else if line.starts_with("import ") {
            &line[7..]
        } else {
            continue;
        };
        let rest = kind.trim_start();
        let Some(q) = rest.chars().next() else {
            continue;
        };
        if q != '\'' && q != '"' {
            continue;
        }
        let inner = &rest[1..];
        let Some(end) = inner.find(q) else { continue };
        let spec = &inner[..end];
        if spec.is_empty() {
            continue;
        }
        // package:<this_pkg>/sub/file.dart → sub/file.dart (in-package)
        let in_pkg_prefix = format!("package:{}/", this_pkg);
        if let Some(stripped) = spec.strip_prefix(&in_pkg_prefix) {
            out.push(stripped.to_string());
            continue;
        }
        // Other package:foo/... → skip (separate dep root)
        if spec.starts_with("package:") || spec.starts_with("dart:") {
            continue;
        }
        // Plain relative path
        out.push(spec.to_string());
    }
    out
}

/// Resolve a Dart relative/in-package export spec to a file on disk.
/// Rejects any path that escapes the package's lib/ root.
fn resolve_dart_relative_path(from_file: &Path, lib_root: &Path, spec: &str) -> Option<PathBuf> {
    let base = if spec.starts_with("../") || spec.starts_with("./") {
        from_file.parent()?.to_path_buf()
    } else {
        // Plain relative like `src/foo.dart` resolves against the file's dir
        // for `export`, but for an in-package `package:<pkg>/src/foo.dart`
        // resolves against lib_root. Try both.
        from_file.parent()?.to_path_buf()
    };
    let candidate = base.join(spec);
    if candidate.is_file() {
        return Some(candidate);
    }
    // Try lib_root-rooted resolution (for in-package absolute specs).
    let from_lib = lib_root.join(spec);
    if from_lib.is_file() {
        return Some(from_lib);
    }
    None
}
