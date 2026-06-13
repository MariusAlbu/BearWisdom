// ---------------------------------------------------------------------------
// Reachability: package entry + bounded export expansion
// ---------------------------------------------------------------------------
//
// dep.root is already the package's `lib/` directory (set at discovery time).
// The main entry is conventionally `lib/<package>.dart`. It commonly uses
// `export 'src/foo.dart';` to re-publish implementation files under
// `lib/src/` — the eager walk skips `src/` entirely, which under-covers types
// that are publicly exported from impl files. The reachability path fixes that
// by starting at the entry and following `export '...';` statements, while
// still skipping anything not re-exported.
//
// A framework entry package's public surface often lives behind a
// cross-package `export 'package:<other>/<file>.dart' show ...;` directive:
// the symbol is *defined* in a transitive package and only reachable through
// a secondary library file of that package that the transitive's own
// conventional entry never re-exports. When the sibling roots map is
// supplied, the walk follows those cross-package specs to the defining leaf,
// labeling each pulled file with its owning package. Bounded by
// `DART_EXPORT_MAX_DEPTH` and a per-walk `seen` set (cycle guard).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::ecosystem::externals::ExternalDepRoot;
use crate::walker::WalkedFile;

// Bounds the export-leaf walk, including cross-package hops. A cross-package
// `export 'package:<other>/...'` consumes one level just like an in-package
// hop, so a framework entry → secondary-library → impl-leaf chain
// (`test` → `matcher/expect.dart` → `src/expect/expect.dart`) fits inside it.
// Matches the transitive-pass cap the npm fixpoint walk uses; the per-walk
// `seen` set + per-package convergence keep the file count bounded well below
// this in practice.
const DART_EXPORT_MAX_DEPTH: u32 = 5;

/// Map of `module_path → lib_root` for every discovered dep in the ecosystem.
/// Lets a cross-package `export 'package:<other>/...'` spec resolve to the
/// other package's on-disk `lib/` directory.
pub(super) type SiblingRoots = HashMap<String, PathBuf>;

/// The package whose export chain is currently being expanded. Carries the
/// module label so files are tagged with their owning package even after a
/// cross-package hop.
struct ActivePackage<'a> {
    module_path: &'a str,
    lib_root: &'a Path,
}

pub(super) fn resolve_dart_package_entry(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let empty = SiblingRoots::new();
    resolve_dart_package_entry_with_siblings(dep, &empty)
}

/// Reachability walk that may cross package boundaries via the `siblings`
/// map. Starts at the conventional `lib/<package>.dart` entry, follows
/// in-package `export`/`part` specs, and — when the export targets another
/// discovered package (`export 'package:<other>/<file>'`) — resolves that
/// package's `lib/` root and continues the walk there.
pub(super) fn resolve_dart_package_entry_with_siblings(
    dep: &ExternalDepRoot,
    siblings: &SiblingRoots,
) -> Vec<WalkedFile> {
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    resolve_dart_package_entry_shared_seen(dep, siblings, &mut seen)
}

/// Same walk with a caller-owned `seen` set, so a pre-pull over many dep
/// roots visits (reads, pushes) each file at most once project-wide instead
/// of once per root — the walks of sibling-reachable packages overlap
/// heavily once cross-package export hops are in play.
pub(super) fn resolve_dart_package_entry_shared_seen(
    dep: &ExternalDepRoot,
    siblings: &SiblingRoots,
    seen: &mut std::collections::HashSet<PathBuf>,
) -> Vec<WalkedFile> {
    let entry = dep.root.join(format!("{}.dart", dep.module_path));
    if !entry.is_file() {
        return Vec::new();
    }

    let mut out = Vec::new();
    let active = ActivePackage {
        module_path: &dep.module_path,
        lib_root: &dep.root,
    };
    expand_dart_exports_into(&active, &entry, siblings, &mut out, seen, 0);
    out
}

fn expand_dart_exports_into(
    active: &ActivePackage<'_>,
    file: &Path,
    siblings: &SiblingRoots,
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

    let rel_sub = match file.strip_prefix(active.lib_root) {
        Ok(p) => p.to_string_lossy().replace('\\', "/"),
        Err(_) => return,
    };
    out.push(WalkedFile {
        relative_path: format!("ext:dart:{}/{}", active.module_path, rel_sub),
        absolute_path: file.to_path_buf(),
        language: "dart",
    });

    if depth >= DART_EXPORT_MAX_DEPTH {
        return;
    }

    let Ok(src) = std::fs::read_to_string(file) else {
        return;
    };

    // In-package specs continue within the active package.
    for spec in extract_dart_exports(&src, active.module_path) {
        let Some(next) = resolve_dart_relative_path(file, active.lib_root, &spec) else {
            continue;
        };
        expand_dart_exports_into(active, &next, siblings, out, seen, depth + 1);
    }

    // Cross-package specs switch the active package to the target's lib root,
    // when that package is among the discovered siblings.
    for (package, sub_path) in extract_dart_cross_package_exports(&src, active.module_path) {
        let Some(lib_root) = siblings.get(&package) else {
            continue;
        };
        let next = lib_root.join(&sub_path);
        if !next.is_file() {
            continue;
        }
        let next_active = ActivePackage {
            module_path: &package,
            lib_root,
        };
        expand_dart_exports_into(&next_active, &next, siblings, out, seen, depth + 1);
    }
}

/// Scan line-oriented for `export '...';` and `export "...";`. Also handles
/// `part '...';` (a file split across multiple files) and `import '...';` with
/// package-internal specifiers. Returns the path-like inner string for
/// relative (`src/foo.dart`) and in-package (`package:<this_pkg>/foo.dart`)
/// specifiers; external `package:other/...` and `dart:` imports are skipped
/// here — cross-package specs are handled by
/// `extract_dart_cross_package_exports`.
pub(super) fn extract_dart_exports(src: &str, this_pkg: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in src.lines() {
        let Some(spec) = directive_spec(raw) else {
            continue;
        };
        // package:<this_pkg>/sub/file.dart → sub/file.dart (in-package)
        let in_pkg_prefix = format!("package:{}/", this_pkg);
        if let Some(stripped) = spec.strip_prefix(&in_pkg_prefix) {
            out.push(stripped.to_string());
            continue;
        }
        // Other package:foo/... → handled cross-package; dart: → skip.
        if spec.starts_with("package:") || spec.starts_with("dart:") {
            continue;
        }
        // Plain relative path
        out.push(spec.to_string());
    }
    out
}

/// Scan for cross-package `export 'package:<other>/<sub>.dart' [show ...];`
/// directives, where `<other>` is NOT the active package. Returns
/// `(package_name, sub_path)` pairs. `sub_path` is the in-`lib/` path of the
/// target file (`expect.dart`, `src/scaffolding.dart`). `dart:` specs and
/// in-package `package:<this_pkg>/...` specs are excluded — the latter are
/// emitted by `extract_dart_exports`.
///
/// `show`/`hide` clauses after the spec are ignored: the walk is
/// file-granular. A `show expect;` pulls the whole exported file; downstream
/// name resolution filters to the shown symbol.
pub(super) fn extract_dart_cross_package_exports(
    src: &str,
    this_pkg: &str,
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for raw in src.lines() {
        // Cross-package hops are RE-EXPORT chains only. A cross-package
        // `import` is a consumer edge, not a published-surface edge —
        // following imports here would expand into the import closure of
        // the entire dependency graph instead of the entry's export tree.
        if !raw.trim_start().starts_with("export ") {
            continue;
        }
        let Some(spec) = directive_spec(raw) else {
            continue;
        };
        let Some(rest) = spec.strip_prefix("package:") else {
            continue;
        };
        let Some((package, sub_path)) = rest.split_once('/') else {
            continue;
        };
        if package.is_empty() || sub_path.is_empty() || package == this_pkg {
            continue;
        }
        out.push((package.to_string(), sub_path.to_string()));
    }
    out
}

/// Extract the quoted specifier from an `export`/`part`/`import` directive
/// line. Returns the inner path-like string, or `None` for non-directive
/// lines or malformed/empty specifiers.
fn directive_spec(raw: &str) -> Option<String> {
    let line = raw.trim_start();
    let after = if let Some(rest) = line.strip_prefix("export ") {
        rest
    } else if let Some(rest) = line.strip_prefix("part ") {
        rest
    } else if let Some(rest) = line.strip_prefix("import ") {
        rest
    } else {
        return None;
    };
    let rest = after.trim_start();
    let q = rest.chars().next()?;
    if q != '\'' && q != '"' {
        return None;
    }
    let inner = &rest[1..];
    let end = inner.find(q)?;
    let spec = &inner[..end];
    if spec.is_empty() {
        None
    } else {
        Some(spec.to_string())
    }
}

/// Resolve a Dart relative/in-package export spec to a file on disk. Rejects
/// any path that escapes the package's lib/ root.
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

#[cfg(test)]
#[path = "reachability_tests.rs"]
mod tests;
