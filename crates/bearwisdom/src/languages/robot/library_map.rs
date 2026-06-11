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

/// Project-wide map: basename of a `.robot`/`.resource` file → list
/// of project-relative full paths sharing that basename. The Robot
/// extractor stores the bare filename from
/// `Resource    atest_resource.robot`; without this map the resolver
/// can't call `lookup.in_file(...)` because indexed files use full
/// paths (`atest/resources/atest_resource.robot`).
///
/// A `Vec` rather than a single `String` because basename collisions
/// are common in test corpora (e.g. robot-framework has two
/// `telnet_resource.robot` files — one in `atest/robot/.../telnet/`
/// and one in `atest/testdata/.../telnet/`). The resolver resolves
/// the ambiguity at the call site by picking the candidate in the
/// importer's directory, falling back to the lexicographically-first
/// when no same-dir match exists.
///
/// Built once per index pass in `build_robot_resource_basename_map`.
pub type RobotResourceBasenameMap = HashMap<String, Vec<String>>;

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
    // Include externally-walked (`ext:`) Python files: a Robot suite's
    // `Library  SeleniumLibrary` resolves to a pip-installed package under
    // site-packages, not just a project-vendored `.py`. `pick_best_match`
    // prefers a project-internal copy when both exist, so a vendored library
    // (e.g. robot-framework's own `src/robot/.../BuiltIn.py`) still wins.
    let py_paths: Vec<&str> = parsed
        .iter()
        .filter(|pf| pf.path.ends_with(".py"))
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
            // Dedup by `(library_name, py_file_path)` pair. Robot libraries
            // can share a `.py` file under different names — `Library
            // libraryscope.Global` and `Library libraryscope.Suite` both
            // resolve to libraryscope.py via the dotted-fallback. Earlier
            // dedup-by-path lost every entry past the first, so qualified
            // calls like `libraryscope.Suite.Should Be Registered` had no
            // matching import to anchor `is_library_import` against.
            if !all
                .iter()
                .any(|l| l.library_name == lib.library_name && l.py_file_path == lib.py_file_path)
            {
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
                    // Dedup by `(library_name, py_file_path)`; multiple
                    // dotted Library imports map to the same `.py` file.
                    if !all.iter().any(|l| {
                        l.library_name == lib.library_name && l.py_file_path == lib.py_file_path
                    }) {
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

/// Collect the package member modules for a resolved library entry point.
///
/// A flat module (`Foo.py`) has no members — returns empty. A package entry
/// (`<pkg>/__init__.py`) returns every other indexed `.py` file under the
/// same package directory, so the dynamic-keyword scan can reach keyword
/// methods scattered across `<pkg>/keywords/*.py`. Member paths are matched
/// by the directory prefix `<dir-of-init>/`, which keeps an unrelated
/// sibling package (`OtherPkg/...`) out even when both live under the same
/// site-packages root.
pub fn package_member_modules<'a>(library_path: &str, parsed: &'a [ParsedFile]) -> Vec<&'a str> {
    let normalized_lib = library_path.replace('\\', "/");
    // The package directory is everything up to (and including) the trailing
    // slash before `__init__.py`. String-sliced rather than via
    // `std::path::Path` so the `ext:py:` colon head doesn't confuse Windows
    // path parsing.
    let Some(pkg_dir) = normalized_lib.strip_suffix("/__init__.py") else {
        return Vec::new();
    };
    let prefix = format!("{pkg_dir}/");
    parsed
        .iter()
        .filter_map(|pf| {
            let p = pf.path.as_str();
            if p == library_path || !p.ends_with(".py") {
                return None;
            }
            pf.path.replace('\\', "/").starts_with(&prefix).then_some(p)
        })
        .collect()
}

/// Collect the distinct `Library  <name>` names declared across project
/// `.robot`/`.resource` files, plus the auto-imported `BuiltIn`.
///
/// These names are the project-driven demand signal for the externals
/// pull: a suite declaring `Library  SeleniumLibrary` needs SeleniumLibrary's
/// site-packages modules walked even though no Python `import` references
/// them. Resource imports (`.robot`/`.resource`) are excluded — those are
/// resolved within the project, not against pip-installed packages.
pub fn collect_declared_library_names(parsed: &[ParsedFile]) -> HashSet<String> {
    let mut names: HashSet<String> = HashSet::new();
    names.insert("BuiltIn".to_string());
    for pf in parsed {
        if pf.path.starts_with("ext:") {
            continue;
        }
        if !pf.path.ends_with(".robot") && !pf.path.ends_with(".resource") {
            continue;
        }
        for r in &pf.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            let raw = r.target_name.as_str();
            if raw.ends_with(".robot") || raw.ends_with(".resource") {
                continue;
            }
            // The last dotted segment is the library/module the keywords
            // come from (`libraryscope.Global` → `libraryscope` is the
            // package; both the leaf and the head are recorded so either
            // shape finds a matching site-packages root).
            if let Some(head) = raw.split('.').next() {
                if !head.is_empty() {
                    names.insert(head.to_string());
                }
            }
            if let Some(leaf) = raw.rsplit('.').next() {
                let leaf = leaf.trim_end_matches(".py");
                if !leaf.is_empty() {
                    names.insert(leaf.to_string());
                }
            }
        }
    }
    names
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
        map.entry(basename.to_string())
            .or_default()
            .push(pf.path.clone());
    }
    // Sort each candidate list lexicographically so the fallback
    // pick is deterministic across runs.
    for paths in map.values_mut() {
        paths.sort();
    }
    map
}

/// Pick the resource path with the closest directory affinity to the
/// importer. Same-directory wins; otherwise the lexicographically-first
/// candidate (already sorted) — keeps the choice stable across runs.
pub fn pick_resource_for_importer<'a>(
    candidates: &'a [String],
    importer_path: &str,
) -> Option<&'a str> {
    if candidates.is_empty() {
        return None;
    }
    let importer_dir = std::path::Path::new(importer_path)
        .parent()
        .map(|p| p.to_string_lossy().replace('\\', "/"));
    if let Some(dir) = importer_dir.as_deref() {
        if let Some(same_dir) = candidates.iter().find(|p| {
            std::path::Path::new(p)
                .parent()
                .map(|d| d.to_string_lossy().replace('\\', "/") == dir)
                .unwrap_or(false)
        }) {
            return Some(same_dir.as_str());
        }
    }
    candidates.first().map(String::as_str)
}

/// Resolve a `Library  <name>` entry to a project `.py` file path.
/// Supports the four forms Robot accepts:
///   * `Library  TestCheckerLibrary`        — bare module name
///   * `Library  KeywordDecorator.py`       — explicit .py file; the
///     name IS already the basename, just match it as-is
///   * `Library  pkg.subpkg.MyLib`          — dotted module path; the
///     leaf segment is the .py basename
///   * `Library  SeleniumLibrary`           — a package whose keyword class
///     lives in `<name>/__init__.py` (DynamicCore aggregators), not a flat
///     `<name>.py`. Resolved to the package `__init__.py` when no flat
///     module matches.
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
    // Package form: `Library  SeleniumLibrary` with the keyword class on
    // `SeleniumLibrary/__init__.py`. Match either segment as a package dir.
    if let Some(p) = pick_package_init_match(last_seg, py_paths) {
        return Some(p);
    }
    if first_seg != last_seg {
        if let Some(p) = pick_package_init_match(first_seg, py_paths) {
            return Some(p);
        }
    }
    None
}

/// Resolve a `Library  <name>` package to its `<name>/__init__.py`.
///
/// A DynamicCore library (`SeleniumLibrary`) is a package directory, not a
/// flat module — the keyword class aggregating `keywords/*.py` is defined in
/// the package `__init__.py`. The site-packages copy surfaces as
/// `ext:py:<name>/<...>/__init__.py`; the directory segment immediately above
/// the matching `__init__.py` must be exactly `<name>`. A project-internal
/// copy wins over the `ext:` twin (same tie-break as `pick_best_match`).
fn pick_package_init_match(package_name: &str, py_paths: &[&str]) -> Option<String> {
    let mut matches: Vec<&str> = py_paths
        .iter()
        .copied()
        .filter(|p| is_package_init_for(p, package_name))
        .collect();
    if matches.is_empty() {
        return None;
    }
    matches.sort();
    if let Some(internal) = matches.iter().find(|p| !p.starts_with("ext:")) {
        return Some((*internal).to_string());
    }
    matches.first().map(|s| (*s).to_string())
}

/// True when `path` ends with `<package_name>/__init__.py` (case-insensitive
/// — Robot/Python import names are case-insensitive on the platforms BW
/// targets). Matched as a `/`-split string rather than via `std::path::Path`:
/// the `ext:py:` virtual paths use `/` separators by construction and the
/// colon-prefixed head confuses `Path::file_name` on Windows.
fn is_package_init_for(path: &str, package_name: &str) -> bool {
    let normalized = path.replace('\\', "/");
    let mut segs = normalized.rsplit('/');
    if segs.next() != Some("__init__.py") {
        return false;
    }
    segs.next()
        .map(|dir| strip_virtual_head(dir).eq_ignore_ascii_case(package_name))
        .unwrap_or(false)
}

/// Drop a leading externals virtual head (`ext:py:`, `ext:ts:`, …) from a
/// path segment. The walker glues the head onto the first segment with no
/// separator, so the top-level package directory of a walked dependency
/// reads as `ext:py:SeleniumLibrary`; the bare directory name is everything
/// after the second colon.
fn strip_virtual_head(segment: &str) -> &str {
    let Some(after_ext) = segment.strip_prefix("ext:") else {
        return segment;
    };
    match after_ext.split_once(':') {
        Some((_lang, rest)) => rest,
        None => segment,
    }
}

/// Resolve a Resource basename (`atest_resource.robot`) to its full
/// project path (`atest/resources/atest_resource.robot`).
fn resolve_basename(basename: &str, importer_path: &str, candidates: &[&str]) -> Option<String> {
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
    let importer_dir = Path::new(importer_path)
        .parent()
        .map(|p| p.to_string_lossy().replace('\\', "/"));
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
    // A project-internal copy is the authoritative target; the site-packages
    // (`ext:`) copy is the fallback. `ext:`-prefixed paths sort before `src/`
    // lexicographically, so an explicit non-`ext:` pass is needed to keep a
    // vendored library winning over its externally-walked twin.
    if let Some(internal) = matches.iter().find(|p| !p.starts_with("ext:")) {
        return Some((*internal).to_string());
    }
    matches.first().map(|s| (*s).to_string())
}
