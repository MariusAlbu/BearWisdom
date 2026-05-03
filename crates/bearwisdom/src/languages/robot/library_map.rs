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

/// Project-wide map: basename of a `.robot`/`.resource` file → its
/// project-relative full path. The Robot extractor stores the bare
/// filename from `Resource    atest_resource.robot`; without this map
/// the resolver can't call `lookup.in_file(...)` because indexed files
/// use full paths (`atest/resources/atest_resource.robot`).
///
/// Built once per index pass in `build_robot_resource_basename_map`.
pub type RobotResourceBasenameMap = HashMap<String, String>;

/// Library names that Robot Framework imports implicitly into every
/// suite/resource, with no explicit `Library  <name>` declaration.
///
/// Per the Robot Framework spec the only implicit library is `BuiltIn`
/// (`Should Be Equal`, `No Operation`, `Set Variable`, `Length Should
/// Be`, ~150 other keywords). The list is pluralised as a convenience
/// in case future spec additions land — adding here is a one-line
/// change and the lookup is O(1) per file at index time.
const AUTO_IMPORTED_LIBRARIES: &[&str] = &["BuiltIn"];

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

    // Robot Framework auto-imports `BuiltIn` for every test/resource file
    // — no `Library  BuiltIn` declaration is required. The keywords it
    // exposes (`Should Be Equal`, `No Operation`, `Set Variable`, ...)
    // would otherwise leak unresolved in any project that vendors the
    // framework's Python source. Find a project-internal `BuiltIn.py`
    // (typically `src/robot/libraries/BuiltIn.py`) and treat it as an
    // implicit library on every robot/resource file.
    //
    // Only auto-injected when the project actually contains a BuiltIn.py;
    // application projects that just use Robot at runtime (where BuiltIn
    // lives in site-packages, not the source tree) are unaffected.
    let auto_libs: Vec<RobotPythonLibrary> = AUTO_IMPORTED_LIBRARIES
        .iter()
        .filter_map(|name| {
            let target = format!("{name}.py");
            let py = py_paths.iter().copied().find(|p| {
                std::path::Path::new(p)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n == target)
                    .unwrap_or(false)
            })?;
            Some(RobotPythonLibrary {
                library_name: (*name).to_string(),
                py_file_path: py.to_string(),
            })
        })
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
    // and accumulate libraries from every visited resource. Auto-imported
    // libraries (BuiltIn, ...) are seeded first so they're available on
    // every file regardless of explicit imports.
    let mut result: RobotLibraryMap = HashMap::new();
    for pf in parsed {
        if pf.path.starts_with("ext:") {
            continue;
        }
        if !pf.path.ends_with(".robot") && !pf.path.ends_with(".resource") {
            continue;
        }
        let mut all: Vec<RobotPythonLibrary> = auto_libs.clone();
        for lib in direct_libs.get(pf.path.as_str()).into_iter().flatten() {
            if !all.iter().any(|l| l.py_file_path == lib.py_file_path) {
                all.push(lib.clone());
            }
        }
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

/// Walk parsed files and build a `basename → full_path` map for every
/// indexed `.robot`/`.resource` file. Used by `RobotResolver` to rewrite
/// `Resource    atest_resource.robot` (basename) into
/// `atest/resources/atest_resource.robot` (full path) before calling
/// `lookup.in_file(...)`.
///
/// When two project files share a basename, the lexicographically-first
/// one wins so the choice is stable across runs. (Real-world projects
/// almost never have basename collisions for resource files; if they do,
/// the right answer needs a same-importer-dir tie-break which the
/// build-time map can't provide.)
pub fn build_robot_resource_basename_map(parsed: &[ParsedFile]) -> RobotResourceBasenameMap {
    let mut map: RobotResourceBasenameMap = HashMap::new();
    for pf in parsed {
        if pf.path.starts_with("ext:") {
            continue;
        }
        if !pf.path.ends_with(".robot") && !pf.path.ends_with(".resource") {
            continue;
        }
        let Some(basename) = std::path::Path::new(&pf.path)
            .file_name()
            .and_then(|n| n.to_str())
        else {
            continue;
        };
        match map.get(basename) {
            None => {
                map.insert(basename.to_string(), pf.path.clone());
            }
            Some(existing) if pf.path < *existing => {
                map.insert(basename.to_string(), pf.path.clone());
            }
            _ => {}
        }
    }
    map
}

/// Resolve a `Library  <name>` entry to a project `.py` file path.
/// Supports the three forms Robot accepts:
///   * `Library  TestCheckerLibrary`        — bare module name
///   * `Library  KeywordDecorator.py`       — explicit .py file; the
///     name IS already the basename, just match it as-is
///   * `Library  pkg.subpkg.MyLib`          — dotted module path; the
///     leaf segment is the .py basename
fn resolve_library_to_py(
    library_name: &str,
    importer_path: &str,
    py_paths: &[&str],
) -> Option<String> {
    if library_name.ends_with(".py") {
        // Already a full basename — use it directly. Stripping the `.py`
        // first would reduce `KeywordDecorator.py` to `py` (the literal
        // last `.`-segment) and search for `py.py`, which never exists.
        return pick_best_match(library_name, importer_path, py_paths);
    }
    // Two interpretations of `pkg.subpkg.MyLib` — try them in order:
    //   1. Last segment IS the module: `MyLib.py`. Common for
    //      `package_name.module_basename`.
    //   2. First segment IS the module, later segments are dotted attr
    //      access into the module's contents: `Library  libraryscope.Global`
    //      means import module `libraryscope`, then keywords come from
    //      class `Global` inside it. The .py file is `libraryscope.py`.
    // We accept either match — Robot itself accepts whichever Python
    // interprets first, and our library_map only needs to find the file
    // so the resolver can flag the call as a known external library.
    let last_seg = library_name.rsplit('.').next().unwrap_or(library_name);
    let last_target = format!("{last_seg}.py");
    if let Some(p) = pick_best_match(&last_target, importer_path, py_paths) {
        return Some(p);
    }
    let first_seg = library_name.split('.').next().unwrap_or(library_name);
    if first_seg != last_seg {
        let first_target = format!("{first_seg}.py");
        if let Some(p) = pick_best_match(&first_target, importer_path, py_paths) {
            return Some(p);
        }
    }
    None
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
///
/// `target_basename` may be a bare basename (`atest_resource.robot`),
/// a relative path (`../runner/cli_resource.robot`), or an absolute
/// path — only the file-name suffix is used for matching.
fn pick_best_match(
    target_basename: &str,
    importer_path: &str,
    candidates: &[&str],
) -> Option<String> {
    let importer_dir = Path::new(importer_path).parent().map(|p| {
        p.to_string_lossy().replace('\\', "/")
    });
    // Normalise the target down to just the file-name suffix. The
    // extractor preserves whatever the user wrote (`../runner/x.robot`),
    // but candidates are full project paths whose basenames never carry
    // leading `../` segments.
    let target_name = Path::new(target_basename)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(target_basename);
    let mut matches: Vec<&str> = candidates
        .iter()
        .copied()
        .filter(|p| {
            Path::new(p)
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.eq_ignore_ascii_case(target_name))
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
