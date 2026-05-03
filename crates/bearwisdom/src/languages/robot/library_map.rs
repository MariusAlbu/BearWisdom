// =============================================================================
// languages/robot/library_map.rs — Robot ↔ Python Library binding map
//
// Robot test files reach Python keyword libraries through a chain of
// `Resource` imports that ends at one or more `Library  <name>` entries.
// Example:
//
//   atest/robot/output/foo.robot
//     Resource    atest_resource.robot
//
//   atest/resources/atest_resource.robot
//     Library     TestCheckerLibrary
//
//   atest/resources/TestCheckerLibrary.py
//     def check_test_case(...):    # ← `Check Test Case` resolves here
//
// The standard Robot keyword resolver only walks one hop, so calls like
// `Check Test Case` evaporate (5,104 of them in robot-framework alone).
//
// This pre-pass builds the closure once per index pass:
//   robot_file_path → Vec<RobotPythonLibrary>
// where each entry names a Python library and the absolute project-relative
// path of the `.py` file the resolver should look in.
// =============================================================================

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::types::{EdgeKind, ParsedFile};

/// One Library binding the resolver can check during a Calls ref lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RobotPythonLibrary {
    /// The bare name as written in `Library  Foo` (or the last dotted
    /// segment of `Library  pkg.subpkg.Foo`). Useful for diagnostics.
    pub library_name: String,
    /// Project-relative path of the `.py` file we resolved the library to.
    /// Always points to a file the parser actually indexed.
    pub py_file_path: String,
}

/// Per-file flattened Library list. `HashMap::get` returns `None` for
/// files that have no transitive Library imports — common for pure unit
/// tests that don't pull in helper resources.
pub type RobotLibraryMap = HashMap<String, Vec<RobotPythonLibrary>>;

/// Walk parsed files, resolve Robot import chains, and return a map
/// keyed by `.robot`/`.resource` file path.
///
/// Resolution rules:
///   * **Library lookup**: take the last `.`-segment of the import name
///     and find a project file `<seg>.py`. If multiple files in the
///     project share the basename, prefer one in the same directory as
///     the importing file; otherwise pick the first lexicographically
///     so the choice is stable across runs.
///   * **Resource lookup**: imports are stored as basenames by the
///     extractor (the parser doesn't resolve relative paths). Match by
///     basename against indexed `.robot`/`.resource` files. Same
///     same-dir → lex tie-break as Library lookup.
///   * **Transitive**: for each robot file, BFS through its Resource
///     imports collecting Library entries from every visited resource.
///     A `visited` set guards against import cycles.
pub fn build_robot_library_map(parsed: &[ParsedFile]) -> RobotLibraryMap {
    let py_paths: Vec<&str> = parsed
        .iter()
        .filter(|pf| !pf.path.starts_with("ext:") && pf.path.ends_with(".py"))
        .map(|pf| pf.path.as_str())
        .collect();
    let robot_paths: Vec<&str> = parsed
        .iter()
        .filter(|pf| {
            !pf.path.starts_with("ext:")
                && (pf.path.ends_with(".robot") || pf.path.ends_with(".resource"))
        })
        .map(|pf| pf.path.as_str())
        .collect();

    // Direct imports per robot/resource file (no transitivity yet).
    let mut direct_libs: HashMap<&str, Vec<RobotPythonLibrary>> = HashMap::new();
    let mut direct_resources: HashMap<&str, Vec<String>> = HashMap::new();

    for pf in parsed {
        if pf.path.starts_with("ext:") {
            continue;
        }
        if !pf.path.ends_with(".robot") && !pf.path.ends_with(".resource") {
            continue;
        }
        let mut libs: Vec<RobotPythonLibrary> = Vec::new();
        let mut resources: Vec<String> = Vec::new();
        for r in &pf.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            let raw = r.target_name.as_str();
            if raw.ends_with(".robot") || raw.ends_with(".resource") {
                if let Some(full) = resolve_basename(raw, &pf.path, &robot_paths) {
                    resources.push(full);
                }
            } else if let Some(py_path) = resolve_library_to_py(raw, &pf.path, &py_paths) {
                libs.push(RobotPythonLibrary {
                    library_name: raw.to_string(),
                    py_file_path: py_path,
                });
            }
        }
        direct_libs.insert(pf.path.as_str(), libs);
        direct_resources.insert(pf.path.as_str(), resources);
    }

    // Transitive closure: for each file, BFS through its Resource imports
    // and accumulate libraries from every visited resource.
    let mut result: RobotLibraryMap = HashMap::new();
    for pf in parsed {
        if pf.path.starts_with("ext:") {
            continue;
        }
        if !pf.path.ends_with(".robot") && !pf.path.ends_with(".resource") {
            continue;
        }
        let mut all: Vec<RobotPythonLibrary> = direct_libs
            .get(pf.path.as_str())
            .cloned()
            .unwrap_or_default();
        let mut visited: HashSet<String> = HashSet::new();
        let mut stack: Vec<String> = direct_resources
            .get(pf.path.as_str())
            .cloned()
            .unwrap_or_default();
        while let Some(res) = stack.pop() {
            if !visited.insert(res.clone()) {
                continue;
            }
            if let Some(libs) = direct_libs.get(res.as_str()) {
                for lib in libs {
                    if !all.iter().any(|l| l.py_file_path == lib.py_file_path) {
                        all.push(lib.clone());
                    }
                }
            }
            if let Some(more) = direct_resources.get(res.as_str()) {
                stack.extend(more.iter().cloned());
            }
        }
        if !all.is_empty() {
            result.insert(pf.path.clone(), all);
        }
    }
    result
}

/// Resolve a `Library  <name>` entry to a project `.py` file path.
/// Supports the two forms Robot accepts:
///   * `Library  TestCheckerLibrary`        — bare module name
///   * `Library  pkg.subpkg.MyLib`          — dotted module path; the
///     leaf segment is the .py basename
fn resolve_library_to_py(
    library_name: &str,
    importer_path: &str,
    py_paths: &[&str],
) -> Option<String> {
    let stem = library_name.rsplit('.').next().unwrap_or(library_name);
    let target = format!("{stem}.py");
    pick_best_match(&target, importer_path, py_paths)
}

/// Resolve a Resource basename (`atest_resource.robot`) to its full
/// project path (`atest/resources/atest_resource.robot`).
fn resolve_basename(
    basename: &str,
    importer_path: &str,
    candidates: &[&str],
) -> Option<String> {
    pick_best_match(basename, importer_path, candidates)
}

/// Pick the candidate file whose basename matches `target_basename`.
/// Prefers a candidate in the same directory as the importer, otherwise
/// returns the lexicographically-first match for deterministic output.
fn pick_best_match(
    target_basename: &str,
    importer_path: &str,
    candidates: &[&str],
) -> Option<String> {
    let importer_dir = Path::new(importer_path).parent().map(|p| {
        p.to_string_lossy().replace('\\', "/")
    });
    let mut matches: Vec<&str> = candidates
        .iter()
        .copied()
        .filter(|p| {
            Path::new(p)
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.eq_ignore_ascii_case(target_basename))
                .unwrap_or(false)
        })
        .collect();
    if matches.is_empty() {
        return None;
    }
    if let Some(dir) = importer_dir.as_deref() {
        if let Some(same_dir) = matches.iter().find(|p| {
            PathBuf::from(p)
                .parent()
                .map(|d| d.to_string_lossy().replace('\\', "/") == dir)
                .unwrap_or(false)
        }) {
            return Some((*same_dir).to_string());
        }
    }
    matches.sort();
    matches.first().map(|s| (*s).to_string())
}
