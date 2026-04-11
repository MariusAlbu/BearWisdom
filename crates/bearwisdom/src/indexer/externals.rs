// =============================================================================
// indexer/externals.rs — external dependency source discovery + walking
//
// Finds the on-disk root of each external dependency declared in a project's
// manifest and enumerates the source files under it. Indexed rows produced
// from these files are written with `origin='external'`, so user-facing
// queries can filter them out while the resolver can still find them.
//
// Currently covers Go only (S3 MVP). Future ecosystems (Python site-packages,
// node_modules, Maven local repo, NuGet global cache) plug in via the same
// shape: discovery → walker → shared pipeline in write::write_parsed_files_with_origin.
// =============================================================================

use std::path::{Path, PathBuf};

use crate::indexer::manifest::go_mod::{find_go_mod, parse_go_mod, GoModDep};
use crate::indexer::manifest::npm::NpmManifest;
use crate::indexer::manifest::pyproject::PyProjectManifest;
use crate::indexer::manifest::ManifestReader;
use crate::walker::WalkedFile;
use tracing::debug;

/// A discovered external dependency root — the directory containing one
/// version of one package on disk.
#[derive(Debug, Clone)]
pub struct ExternalDepRoot {
    /// Canonical module path (e.g., "github.com/gin-gonic/gin").
    pub module_path: String,
    /// Semantic version string as it appears in go.mod (e.g., "v1.9.1").
    pub version: String,
    /// Absolute path to the module cache directory on disk.
    pub root: PathBuf,
    /// Ecosystem identifier. "go" for now.
    pub ecosystem: &'static str,
}

// ---------------------------------------------------------------------------
// Go module cache discovery
// ---------------------------------------------------------------------------

/// Discover all external Go dependency roots for a project.
///
/// Strategy: parse `go.mod`, resolve each direct `require` entry to
/// `$GOMODCACHE/{escaped_module_path}@{version}`, and return the entries
/// whose directory actually exists on disk.
///
/// **Indirect deps are walked only when user code imports them.** A
/// lightweight string scan over `.go` files produces a set of imported
/// module paths; any `go.mod // indirect` entry whose module path (or
/// a prefix of it) appears in that set gets walked too. This catches
/// legitimate transitive exposure — e.g. when a direct dep re-exports
/// types from a transitive dep and user code references those types
/// directly — without the 10-20x symbol table explosion that walking
/// every indirect dep would cause in projects like Gitea (314 indirect
/// vs 14 direct). Indirect deps that user code never touches are still
/// skipped.
pub fn discover_go_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    let Some(go_mod_path) = find_go_mod(project_root) else {
        return Vec::new();
    };
    let Ok(content) = std::fs::read_to_string(&go_mod_path) else {
        return Vec::new();
    };
    let parsed = parse_go_mod(&content);

    let cache_root = match gomodcache_root() {
        Some(p) => p,
        None => {
            debug!("No GOMODCACHE / GOPATH detected; skipping Go externals");
            return Vec::new();
        }
    };

    // For indirect deps, only walk those that user code actually imports.
    // This catches genuine transitive type exposure (e.g., a direct dep
    // re-exports types from a transitive) without the 10-20x symbol table
    // explosion that walking every indirect dep causes in projects like
    // Gitea (314 indirect vs 14 direct). The user-imports set is built by
    // a lightweight string scan over .go files — no tree-sitter needed.
    let user_imports = collect_go_imports(project_root);

    let mut roots = Vec::new();
    for dep in &parsed.require_deps {
        if dep.indirect && !go_dep_is_imported(&dep.path, &user_imports) {
            continue;
        }
        if let Some(root) = resolve_go_dep_path(&cache_root, dep) {
            roots.push(ExternalDepRoot {
                module_path: dep.path.clone(),
                version: dep.version.clone(),
                root,
                ecosystem: "go",
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

/// Scan `.go` files under `project_root` for import strings. Returns a
/// set of unique import paths. Skips vendor/, tests, and common build
/// output directories. This is a lightweight string search — importing
/// tree-sitter here would be overkill for what amounts to a regex.
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
    if depth > 10 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if let Ok(ft) = entry.file_type() {
            if ft.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if matches!(
                        name,
                        ".git" | "vendor" | "node_modules" | "target"
                            | "build" | "dist" | "testdata"
                    ) {
                        continue;
                    }
                }
                scan_go_imports_recursive(&path, out, depth + 1);
            } else if ft.is_file() {
                let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                if !name.ends_with(".go") || name.ends_with("_test.go") {
                    continue;
                }
                let Ok(content) = std::fs::read_to_string(&path) else {
                    continue;
                };
                extract_imports_from_go_source(&content, out);
            }
        }
    }
}

/// Parse an `import "..."` or block `import (\n"..."\n)` from Go source.
/// Intentionally loose — we'd rather include a false positive path string
/// from a comment than miss a real import and under-count transitives.
/// The downstream filter `go_dep_is_imported` uses prefix matching so a
/// stray false positive is inert unless it exactly matches a go.mod dep.
fn extract_imports_from_go_source(
    content: &str,
    out: &mut std::collections::HashSet<String>,
) {
    enum Mode {
        Top,
        InBlock,
    }
    let mut mode = Mode::Top;
    for line in content.lines() {
        let trimmed = line.trim();
        match mode {
            Mode::Top => {
                if trimmed.starts_with("import (") {
                    mode = Mode::InBlock;
                    continue;
                }
                if let Some(rest) = trimmed.strip_prefix("import ") {
                    let rest = rest.trim_start_matches('_').trim();
                    // Support aliased imports: `import foo "..."`
                    let quoted = rest
                        .rsplit_once('"')
                        .map(|(head, _)| head)
                        .and_then(|head| head.rsplit_once('"').map(|(_, s)| s));
                    if let Some(path) = quoted {
                        if !path.is_empty() {
                            out.insert(path.to_string());
                        }
                    }
                }
            }
            Mode::InBlock => {
                if trimmed == ")" {
                    mode = Mode::Top;
                    continue;
                }
                // Line might look like: `"fmt"`, `foo "github.com/x/y"`,
                // `_ "database/sql/driver"`. Extract the first quoted string.
                let bytes = trimmed.as_bytes();
                let first = bytes.iter().position(|&b| b == b'"');
                let Some(start) = first else { continue };
                let after = &trimmed[start + 1..];
                let Some(end_rel) = after.find('"') else { continue };
                let path = &after[..end_rel];
                if !path.is_empty() {
                    out.insert(path.to_string());
                }
            }
        }
    }
}

/// Does any user import path start with the given dep module path?
/// Go imports subpackages (`github.com/foo/bar/subpkg`) of declared
/// modules (`github.com/foo/bar`), so prefix matching is the right test.
fn go_dep_is_imported(
    dep_path: &str,
    user_imports: &std::collections::HashSet<String>,
) -> bool {
    if user_imports.contains(dep_path) {
        return true;
    }
    let prefix = format!("{dep_path}/");
    user_imports.iter().any(|imp| imp.starts_with(&prefix))
}

/// Resolve the on-disk location of `$GOMODCACHE`.
///
/// Order: `$GOMODCACHE` env → `$GOPATH/pkg/mod` → `$HOME/go/pkg/mod`
/// (or `$USERPROFILE\go\pkg\mod` on Windows).
pub fn gomodcache_root() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("GOMODCACHE") {
        let p = PathBuf::from(explicit);
        if p.is_dir() {
            return Some(p);
        }
    }
    if let Some(gopath) = std::env::var_os("GOPATH") {
        // GOPATH may be a colon/semicolon-separated list; use the first entry.
        let first = PathBuf::from(gopath)
            .to_string_lossy()
            .split(|c| c == ':' || c == ';')
            .next()
            .map(PathBuf::from);
        if let Some(p) = first {
            let candidate = p.join("pkg").join("mod");
            if candidate.is_dir() {
                return Some(candidate);
            }
        }
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let candidate = PathBuf::from(home).join("go").join("pkg").join("mod");
    if candidate.is_dir() {
        Some(candidate)
    } else {
        None
    }
}

/// Build the cache directory path for one `require` entry.
///
/// Go applies case-escaping to module paths: every uppercase letter becomes
/// `!<lowercase>`. For example, `github.com/Microsoft/go-winio` lives at
/// `github.com/!microsoft/go-winio@v0.6.2`.
fn resolve_go_dep_path(cache_root: &Path, dep: &GoModDep) -> Option<PathBuf> {
    let escaped = escape_module_path(&dep.path);
    let dirname = format!("{}@{}", escaped, dep.version);
    // The escaped module path may contain `/` which maps to real subdirs.
    let candidate = cache_root.join(dirname.replace('/', std::path::MAIN_SEPARATOR_STR));
    if candidate.is_dir() {
        return Some(candidate);
    }
    // Fall back: split escaped path and join the final segment with the version.
    // e.g., github.com/foo/bar → github.com/foo/bar@v1.2.3
    let mut segments: Vec<&str> = escaped.split('/').collect();
    let last = segments.pop()?;
    let mut path = cache_root.to_path_buf();
    for seg in segments {
        path.push(seg);
    }
    path.push(format!("{last}@{}", dep.version));
    if path.is_dir() {
        Some(path)
    } else {
        None
    }
}

/// Apply Go module path case escaping: uppercase → `!lowercase`.
fn escape_module_path(path: &str) -> String {
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

// ---------------------------------------------------------------------------
// External file walker
// ---------------------------------------------------------------------------

/// Walk one external dependency root and emit `WalkedFile` entries for every
/// source file the indexer knows how to parse.
///
/// File filtering rules (Go-specific for now):
/// - Only `.go` files.
/// - Skip `*_test.go` — test files aren't part of the public API surface.
/// - Skip `internal/testdata/` and `vendor/` subtrees.
///
/// `relative_path` on the returned entries is given as
/// `ext:{module_path}@{version}/{sub_path}` so external and internal files
/// never collide in the files.path unique index.
pub fn walk_external_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_dir(&dep.root, &dep.root, dep, &mut out);
    out
}

fn walk_dir(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if file_type.is_dir() {
            // Skip vendored / test data subtrees.
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "vendor" | "testdata" | ".git" | "_examples") {
                    continue;
                }
            }
            walk_dir(&path, root, dep, out);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !name.ends_with(".go") {
                continue;
            }
            if name.ends_with("_test.go") {
                continue;
            }

            // Build the virtual path: ext:{module}@{version}/{sub_path}
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            let virtual_path = format!("ext:{}@{}/{}", dep.module_path, dep.version, rel_sub);

            out.push(WalkedFile {
                relative_path: virtual_path,
                absolute_path: path,
                language: "go",
            });
        }
    }
}

// ===========================================================================
// PYTHON — site-packages discovery + walker
// ===========================================================================

/// Discover all external Python dependency roots for a project.
///
/// Strategy:
/// 1. Read pyproject.toml via the existing `PyProjectManifest` reader.
/// 2. Locate site-packages via (in order) `BEARWISDOM_PYTHON_SITE_PACKAGES`
///    env override, project-local `.venv` / `venv` / `.env`, or `PYTHONHOME`.
/// 3. For each declared dep, normalize the name (strip extras + version,
///    lowercase, dash→underscore) and probe site-packages for a directory
///    or single-file module with that name.
///
/// No dist-info/top_level.txt reading in the MVP — directory-name matching
/// covers the common case (fastapi, pydantic, sqlalchemy, django). Packages
/// with import names that diverge from the dist name (PyYAML→yaml,
/// python-jose→jose) are misses; fix with dist-info lookup in a later pass.
pub fn discover_python_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    let manifest = PyProjectManifest;
    let Some(data) = manifest.read(project_root) else {
        return Vec::new();
    };
    if data.dependencies.is_empty() {
        return Vec::new();
    }

    let site_packages = find_python_site_packages(project_root);
    if site_packages.is_empty() {
        debug!("No Python site-packages discovered; skipping Python externals");
        return Vec::new();
    }
    debug!(
        "Probing {} Python site-packages root(s) for {} declared deps",
        site_packages.len(),
        data.dependencies.len()
    );

    let mut roots = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

    for dep_raw in &data.dependencies {
        let normalized = normalize_python_dep_name(dep_raw);
        if normalized.is_empty() {
            continue;
        }

        let mut matched = false;
        for sp in &site_packages {
            // Package directory: site-packages/{normalized}/__init__.py or similar.
            let pkg_dir = sp.join(&normalized);
            if pkg_dir.is_dir() && !seen.contains(&pkg_dir) {
                seen.insert(pkg_dir.clone());
                roots.push(ExternalDepRoot {
                    module_path: normalized.clone(),
                    version: String::from("unknown"),
                    root: pkg_dir,
                    ecosystem: "python",
                });
                matched = true;
                break;
            }
            // Single-file module: `site-packages/{normalized}.py`.
            // Packages like `six`, `typing_extensions`, `packaging` ship
            // as one top-level file. Point the root at the file itself;
            // `walk_python_external_root` handles the single-file case
            // by emitting exactly that one WalkedFile entry.
            let file = sp.join(format!("{normalized}.py"));
            if file.is_file() && !seen.contains(&file) {
                seen.insert(file.clone());
                roots.push(ExternalDepRoot {
                    module_path: normalized.clone(),
                    version: String::from("unknown"),
                    root: file,
                    ecosystem: "python",
                });
                matched = true;
                break;
            }
        }

        // Fallback: dist-info/top_level.txt lookup. Covers packages whose
        // distribution name differs from the import name, e.g.
        // `PyYAML` → `yaml`, `python-jose` → `jose`, `Pillow` → `PIL`,
        // `beautifulsoup4` → `bs4`, `opencv-python` → `cv2`.
        //
        // Strategy: for each site-packages dir, scan `.dist-info/` entries
        // whose name starts with the normalized dep name (plus any version
        // suffix), read `top_level.txt` inside, and resolve each listed
        // top-level import to a package directory in the same site-packages.
        if !matched {
            for sp in &site_packages {
                if let Some(roots_from_top_level) =
                    python_top_level_lookup(sp, &normalized, &mut seen)
                {
                    roots.extend(roots_from_top_level);
                    break;
                }
            }
        }
    }
    roots
}

/// Look up `top_level.txt` in every `.dist-info/` whose directory name
/// matches the normalized dependency, and resolve each listed top-level
/// module to a concrete package directory under the same site-packages.
///
/// Returns `None` if no matching dist-info was found, or an empty vector
/// if the dist-info exists but `top_level.txt` is missing or empty — the
/// caller can distinguish "keep looking in other site-packages" from
/// "this dep resolved but had nothing to walk".
fn python_top_level_lookup(
    site_packages: &Path,
    normalized: &str,
    seen: &mut std::collections::HashSet<PathBuf>,
) -> Option<Vec<ExternalDepRoot>> {
    let entries = std::fs::read_dir(site_packages).ok()?;
    let lower_prefix = normalized.to_lowercase();

    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_dir() {
            continue;
        }
        let name_os = entry.file_name();
        let name = name_os.to_string_lossy();
        if !name.ends_with(".dist-info") {
            continue;
        }
        // Dist-info names look like `{Dist_Name}-{version}.dist-info`. The
        // Dist_Name has `-` replaced with `_` per PEP 503 for the directory
        // form. Compare case-insensitively against `normalized`.
        let stem = name.trim_end_matches(".dist-info");
        let dist_part = stem.rsplit_once('-').map(|(d, _)| d).unwrap_or(stem);
        let dist_lower = dist_part.to_lowercase();
        if dist_lower != lower_prefix {
            continue;
        }

        let top_level_path = entry.path().join("top_level.txt");
        let Ok(contents) = std::fs::read_to_string(&top_level_path) else {
            debug!(
                "dist-info {} has no top_level.txt — nothing to walk",
                entry.path().display()
            );
            return Some(Vec::new());
        };

        let mut out = Vec::new();
        for line in contents.lines() {
            let import_name = line.trim();
            if import_name.is_empty() || import_name.starts_with('_') {
                // Skip `_cffi_*` style implementation shims.
                continue;
            }
            let pkg_dir = site_packages.join(import_name);
            if pkg_dir.is_dir() && !seen.contains(&pkg_dir) {
                seen.insert(pkg_dir.clone());
                out.push(ExternalDepRoot {
                    module_path: import_name.to_string(),
                    version: String::from("unknown"),
                    root: pkg_dir,
                    ecosystem: "python",
                });
                continue;
            }
            let single_file = site_packages.join(format!("{import_name}.py"));
            if single_file.is_file() && !seen.contains(&single_file) {
                seen.insert(single_file.clone());
                out.push(ExternalDepRoot {
                    module_path: import_name.to_string(),
                    version: String::from("unknown"),
                    root: single_file,
                    ecosystem: "python",
                });
            }
        }
        return Some(out);
    }
    None
}

/// Locate Python site-packages directories to scan for the given project.
///
/// Order of preference:
/// 1. `BEARWISDOM_PYTHON_SITE_PACKAGES` env var — explicit override, may be
///    a single path or a `;`/`:`-separated list.
/// 2. Project-local venv: `.venv`, `venv`, `.env` with both Windows
///    (`Lib/site-packages`) and Unix (`lib/python*/site-packages`) layouts.
/// 3. `PYTHONHOME` env var pointing at a Python install.
///
/// Returns all discovered paths, not just the first — different ecosystems
/// (editable installs, system + user) may legitimately split packages.
pub fn find_python_site_packages(project_root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut push_if_dir = |p: PathBuf, out: &mut Vec<PathBuf>| {
        if p.is_dir() && !out.contains(&p) {
            out.push(p);
        }
    };

    // 1. Explicit override. `std::env::split_paths` handles the platform
    // separator correctly (`;` on Windows, `:` on Unix) so Windows drive
    // letters like `C:\...` aren't chopped on the colon.
    if let Some(raw) = std::env::var_os("BEARWISDOM_PYTHON_SITE_PACKAGES") {
        for seg in std::env::split_paths(&raw) {
            if seg.as_os_str().is_empty() {
                continue;
            }
            push_if_dir(seg, &mut out);
        }
        if !out.is_empty() {
            return out;
        }
    }

    // 2. Project-local venvs. Check the project root first, then every
    // immediate subdirectory (common monorepo pattern: `backend/.venv`,
    // `api/.venv`, `server/.venv`). Deeper scanning is intentionally
    // avoided — we don't want to pick up nested venvs in vendored third-
    // party projects or test fixtures.
    let mut candidate_dirs: Vec<PathBuf> = vec![project_root.to_path_buf()];
    if let Ok(entries) = std::fs::read_dir(project_root) {
        for entry in entries.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let name = entry.file_name();
            let name_lossy = name.to_string_lossy();
            // Skip the venv dirs themselves and common non-project dirs.
            if name_lossy.starts_with('.')
                || matches!(
                    name_lossy.as_ref(),
                    "node_modules" | "target" | "dist" | "build" | "__pycache__"
                )
            {
                continue;
            }
            candidate_dirs.push(entry.path());
        }
    }

    for dir in &candidate_dirs {
        for venv_name in &[".venv", "venv", ".env"] {
            let venv = dir.join(venv_name);
            if !venv.is_dir() {
                continue;
            }

            // Windows layout: {venv}/Lib/site-packages
            push_if_dir(venv.join("Lib").join("site-packages"), &mut out);

            // Unix layout: {venv}/lib/python{ver}/site-packages
            let unix_lib = venv.join("lib");
            if let Ok(entries) = std::fs::read_dir(&unix_lib) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    if name.starts_with("python") {
                        push_if_dir(entry.path().join("site-packages"), &mut out);
                    }
                }
            }
        }
    }

    if !out.is_empty() {
        return out;
    }

    // 3. PYTHONHOME fallback.
    if let Some(home) = std::env::var_os("PYTHONHOME") {
        let base = PathBuf::from(home);
        push_if_dir(base.join("Lib").join("site-packages"), &mut out);
        // Unix Python home layout.
        if let Ok(entries) = std::fs::read_dir(base.join("lib")) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.starts_with("python") {
                    push_if_dir(entry.path().join("site-packages"), &mut out);
                }
            }
        }
    }

    out
}

/// Normalize a pyproject dependency specifier to a site-packages import name.
///
/// Examples:
/// - `fastapi[standard]<1.0.0,>=0.114.2` → `fastapi`
/// - `pydantic-settings>=2.2.1` → `pydantic_settings`
/// - `psycopg[binary]<4.0.0` → `psycopg`
/// - `SQLAlchemy>=2.0` → `sqlalchemy`
///
/// The normalized form matches the directory name Python writes in
/// site-packages, which follows PEP 503 with hyphens → underscores.
pub fn normalize_python_dep_name(raw: &str) -> String {
    // Strip everything from the first version/extras/marker character.
    let end = raw
        .find(|c: char| {
            matches!(
                c,
                '[' | '<' | '>' | '=' | '!' | '~' | ';' | ' ' | '\t' | '@'
            )
        })
        .unwrap_or(raw.len());
    let name = &raw[..end];
    name.trim()
        .to_lowercase()
        .replace('-', "_")
        .replace('.', "_")
}

/// Walk one Python external dep root and emit `WalkedFile` entries.
///
/// File filtering rules:
/// - Only `.py` files.
/// - Skip `__pycache__/`, `tests/`, `test/`.
/// - Skip `test_*.py` and `*_test.py`.
/// - Skip files under `.dist-info/` or `.egg-info/`.
///
/// Virtual relative_path is `ext:py:{package}/{sub_path}` so externals
/// never collide with internal file paths.
///
/// Handles both directory roots (regular packages with `__init__.py`)
/// and single-file roots (`six.py`, `typing_extensions.py`). For
/// single-file roots, emits exactly one WalkedFile entry with an
/// empty sub-path.
pub fn walk_python_external_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    if dep.root.is_file() {
        // Single-file module: one WalkedFile, no recursion.
        let file_name = dep
            .root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("module.py");
        let virtual_path = format!("ext:py:{}/{}", dep.module_path, file_name);
        out.push(WalkedFile {
            relative_path: virtual_path,
            absolute_path: dep.root.clone(),
            language: "python",
        });
    } else {
        walk_python_dir(&dep.root, &dep.root, dep, &mut out);
    }
    out
}

fn walk_python_dir(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(
                    name,
                    "__pycache__" | "tests" | "test" | ".git" | "_test"
                ) {
                    continue;
                }
                if name.ends_with(".dist-info") || name.ends_with(".egg-info") {
                    continue;
                }
            }
            walk_python_dir(&path, root, dep, out);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !name.ends_with(".py") {
                continue;
            }
            if name.starts_with("test_") || name.ends_with("_test.py") || name == "conftest.py" {
                continue;
            }

            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            let virtual_path = format!("ext:py:{}/{}", dep.module_path, rel_sub);

            out.push(WalkedFile {
                relative_path: virtual_path,
                absolute_path: path,
                language: "python",
            });
        }
    }
}

// ===========================================================================
// TYPESCRIPT / JAVASCRIPT — node_modules discovery + walker
// ===========================================================================

/// Discover all external TypeScript/JavaScript dependency roots for a project.
///
/// Strategy:
/// 1. Read package.json(s) via the existing `NpmManifest` reader (already
///    walks subdirs and handles dependencies/devDependencies/peerDependencies
///    plus Node.js builtins).
/// 2. Locate node_modules via (in order) `BEARWISDOM_TS_NODE_MODULES` env
///    override, project-local `node_modules` at root and immediate subdirs
///    (monorepo layout).
/// 3. For each declared dep, resolve to `node_modules/{name}/` (scoped
///    packages like `@tanstack/react-query` map to `node_modules/@tanstack/react-query/`).
/// 4. Skip Node.js builtins — they don't have an on-disk source tree. The
///    NpmManifest reader adds them to the dep set but they don't exist
///    under node_modules.
/// 5. Skip packages whose directory is missing (not installed).
pub fn discover_ts_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    let manifest = NpmManifest;
    let Some(data) = manifest.read(project_root) else {
        return Vec::new();
    };
    if data.dependencies.is_empty() {
        return Vec::new();
    }

    let node_modules_roots = find_node_modules(project_root);
    if node_modules_roots.is_empty() {
        debug!("No node_modules dirs discovered; skipping TS externals");
        return Vec::new();
    }
    debug!(
        "Probing {} node_modules root(s) for {} declared deps",
        node_modules_roots.len(),
        data.dependencies.len()
    );

    // Node builtins — these are declared as deps by NpmManifest but have no
    // on-disk source under node_modules. Skip them entirely; language
    // resolvers already route them via test_globals / runtime globals.
    let builtins: std::collections::HashSet<&'static str> = [
        "assert", "buffer", "child_process", "cluster", "console", "crypto",
        "dgram", "dns", "domain", "events", "fs", "http", "http2", "https",
        "inspector", "module", "net", "node", "os", "path", "perf_hooks",
        "process", "punycode", "querystring", "readline", "repl", "stream",
        "string_decoder", "timers", "tls", "trace_events", "tty", "url",
        "util", "v8", "vm", "wasi", "worker_threads", "zlib",
    ]
    .into_iter()
    .collect();

    let mut roots = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

    for dep in &data.dependencies {
        if builtins.contains(dep.as_str()) {
            continue;
        }
        // Skip bare scope sentinels (`@tanstack`) — NpmManifest inserts these
        // alongside the real scoped package names; resolving them as a package
        // would incorrectly pull in the whole scope directory.
        if dep.starts_with('@') && !dep.contains('/') {
            continue;
        }
        // Skip `@types/X` entries (DefinitelyTyped type-only packages). The
        // real package `X` is typically also in deps, and the fallback probe
        // below will pull `node_modules/@types/X/` under the `X` module path
        // so its symbols get qualified as `X.Foo` — which is what the TS
        // resolver looks for on `import { Foo } from 'X'`. If we processed
        // `@types/X` as its own dep it would land as `@types/X.Foo` and
        // never match a user-code import.
        if dep.starts_with("@types/") {
            continue;
        }

        // Collect every on-disk root for this dep:
        //
        // 1. The primary `node_modules/{dep}/` directory (which may ship only
        //    `.js` if the package is untyped).
        // 2. The DefinitelyTyped sibling that carries `.d.ts` for untyped
        //    libraries:
        //    - Unscoped: `node_modules/@types/{dep}/`
        //    - Scoped:   `node_modules/@types/{scope}__{name}/`
        //      e.g. `@tanstack/react-query` → `@types/tanstack__react-query`.
        //      This is the escape scheme DefinitelyTyped uses on npm because
        //      `@` is not allowed inside an `@types/*` sub-path.
        //
        // Both roots share the same `module_path` so their symbols get the
        // same package prefix (`react.ReactNode`), and the Tier 1 TS
        // resolver's `{import_module}.{target}` lookup finds them equally.
        let mut pkg_roots: Vec<PathBuf> = Vec::new();
        for nm_root in &node_modules_roots {
            // Scoped package: `@foo/bar` → `node_modules/@foo/bar/`
            // Unscoped: `react` → `node_modules/react/`
            let primary = nm_root.join(dep);
            if primary.is_dir() {
                pkg_roots.push(primary);
            }
            // DefinitelyTyped fallback — unscoped and scoped both.
            if !dep.starts_with('@') {
                let types_dir = nm_root.join("@types").join(dep);
                if types_dir.is_dir() {
                    pkg_roots.push(types_dir);
                }
            } else if let Some(escaped) = definitely_typed_scoped_name(dep) {
                let types_dir = nm_root.join("@types").join(&escaped);
                if types_dir.is_dir() {
                    pkg_roots.push(types_dir);
                }
            }
        }

        for pkg_dir in pkg_roots {
            if seen.insert(pkg_dir.clone()) {
                roots.push(ExternalDepRoot {
                    module_path: dep.clone(),
                    version: String::from("unknown"),
                    root: pkg_dir,
                    ecosystem: "typescript",
                });
            }
        }
    }
    roots
}

/// Convert a scoped npm package name into the DefinitelyTyped escape form.
///
/// DefinitelyTyped publishes types for scoped packages at
/// `@types/{scope}__{name}` because npm disallows nested `@` inside a scope
/// path. For example:
///
/// - `@tanstack/react-query` → `tanstack__react-query`
/// - `@radix-ui/react-dialog` → `radix-ui__react-dialog`
///
/// Returns `None` if the input isn't a scoped name (`@scope/name`) so callers
/// can skip the probe for unscoped deps, which take a different code path.
fn definitely_typed_scoped_name(dep: &str) -> Option<String> {
    let rest = dep.strip_prefix('@')?;
    let (scope, name) = rest.split_once('/')?;
    if scope.is_empty() || name.is_empty() {
        return None;
    }
    Some(format!("{scope}__{name}"))
}

/// Locate node_modules directories for the given project.
///
/// Order of preference:
/// 1. `BEARWISDOM_TS_NODE_MODULES` env override (platform-separated list).
/// 2. `{project_root}/node_modules` (most common).
/// 3. Immediate subdirs of project_root (monorepo pattern: `frontend/`,
///    `packages/`, `apps/`, etc. — same walk shape used for Python venvs).
pub fn find_node_modules(project_root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut push_if_dir = |p: PathBuf, out: &mut Vec<PathBuf>| {
        if p.is_dir() && !out.contains(&p) {
            out.push(p);
        }
    };

    if let Some(raw) = std::env::var_os("BEARWISDOM_TS_NODE_MODULES") {
        for seg in std::env::split_paths(&raw) {
            if seg.as_os_str().is_empty() {
                continue;
            }
            push_if_dir(seg, &mut out);
        }
        if !out.is_empty() {
            return out;
        }
    }

    // Root-level node_modules.
    push_if_dir(project_root.join("node_modules"), &mut out);

    // Immediate subdirs — covers `frontend/node_modules`, `apps/web/node_modules`,
    // `packages/foo/node_modules`, etc. Same monorepo-friendly walk shape as
    // `find_python_site_packages`.
    if let Ok(entries) = std::fs::read_dir(project_root) {
        for entry in entries.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let name = entry.file_name();
            let name_lossy = name.to_string_lossy();
            if name_lossy.starts_with('.')
                || matches!(
                    name_lossy.as_ref(),
                    "node_modules" | "target" | "dist" | "build" | ".turbo" | ".next"
                )
            {
                continue;
            }
            push_if_dir(entry.path().join("node_modules"), &mut out);
        }
    }

    out
}

/// Walk one TS external dep root and emit `WalkedFile` entries.
///
/// File filtering rules:
/// - Include `.ts`, `.tsx`, `.d.ts`, `.mts`, `.cts`, `.d.mts`, `.d.cts`.
/// - Skip `.js`/`.jsx`/`.mjs` — type info for those packages lives in
///   sibling `.d.ts` files that we'll pick up anyway.
/// - Skip `node_modules/` subtrees (nested deps).
/// - Skip `__tests__/`, `test/`, `tests/`, `__mocks__/`, `docs/`,
///   `example/`, `examples/`, `_examples/`, `.storybook/`, `fixtures/`.
/// - Skip `*.test.*`, `*.spec.*`, `*.stories.*`, `*.bench.*`, `*.fixture.*`.
///
/// Virtual relative_path is `ext:ts:{package}/{sub_path}`.
pub fn walk_ts_external_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_ts_dir(&dep.root, &dep.root, dep, &mut out);
    out
}

fn walk_ts_dir(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>) {
    // Nested `node_modules/` trees are skipped by default because pnpm,
    // yarn-workspaces, and legacy hoisting can produce deep duplicate
    // installs of the same package — walking them indexes the same
    // symbols N times and inflates index time by 5-10x on large monorepos.
    //
    // Opt-in via `BEARWISDOM_TS_WALK_NESTED=1` for projects where nested
    // deps genuinely differ from the top level (version conflicts that
    // can't be hoisted, non-portable native deps, pnpm stores). The flag
    // is read once per walk call — cheap enough to not need caching.
    let walk_nested = std::env::var_os("BEARWISDOM_TS_WALK_NESTED")
        .map(|v| v != "0" && !v.is_empty())
        .unwrap_or(false);

    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                // `node_modules` is conditionally skipped based on the
                // opt-in flag. All other skipped dirs (tests, docs, etc.)
                // stay unconditional because they never carry useful
                // public API types.
                if name == "node_modules" && !walk_nested {
                    continue;
                }
                if matches!(
                    name,
                    "__tests__"
                        | "__mocks__"
                        | "test"
                        | "tests"
                        | "docs"
                        | "example"
                        | "examples"
                        | "_examples"
                        | "fixtures"
                        | ".storybook"
                        | ".git"
                ) {
                    continue;
                }
            }
            walk_ts_dir(&path, root, dep, out);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !is_ts_source_file(name) {
                continue;
            }
            if is_test_or_story_file(name) {
                continue;
            }

            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            let virtual_path = format!("ext:ts:{}/{}", dep.module_path, rel_sub);
            // Tree-sitter TS grammar handles `.d.ts` transparently. `.tsx`
            // needs the TSX-specific grammar, which the language registry
            // routes via the `tsx` language id.
            let language = if name.ends_with(".tsx") {
                "tsx"
            } else {
                "typescript"
            };
            out.push(WalkedFile {
                relative_path: virtual_path,
                absolute_path: path,
                language,
            });
        }
    }
}

fn is_ts_source_file(name: &str) -> bool {
    // .d.ts / .d.mts / .d.cts handled by the generic .ts / .mts / .cts suffix match.
    name.ends_with(".ts")
        || name.ends_with(".tsx")
        || name.ends_with(".mts")
        || name.ends_with(".cts")
}

/// Prefix every symbol's `qualified_name` (and `scope_path`) in a parsed
/// TypeScript external file with the owning package name.
///
/// TypeScript declaration files don't carry a package-level scope, so the
/// extractor yields bare qualified names like `Button` or `Button.render`.
/// To make these look up cleanly by `{import_module}.{target}` (which is what
/// the TS Tier 1 resolver tries), rewrite them to `fake-ui.Button` /
/// `fake-ui.Button.render`.
///
/// Mutates the parsed file in place. Idempotent: already-prefixed names are
/// left alone.
pub fn prefix_ts_external_symbols(pf: &mut crate::types::ParsedFile, package: &str) {
    if package.is_empty() {
        return;
    }
    let prefix = format!("{package}.");
    for sym in &mut pf.symbols {
        if !sym.qualified_name.starts_with(&prefix) {
            sym.qualified_name = format!("{prefix}{}", sym.qualified_name);
        }
        sym.scope_path = match sym.scope_path.take() {
            Some(sp) if !sp.starts_with(&prefix) => Some(format!("{prefix}{sp}")),
            Some(sp) => Some(sp),
            None => Some(package.to_string()),
        };
    }
}

fn is_test_or_story_file(name: &str) -> bool {
    // Look for `.test.`, `.spec.`, `.stories.`, `.bench.`, `.fixture.` anywhere
    // before the extension.
    let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name);
    stem.ends_with(".test")
        || stem.ends_with(".spec")
        || stem.ends_with(".stories")
        || stem.ends_with(".bench")
        || stem.ends_with(".fixture")
        || stem == "test"
        || stem == "index.test"
}

// ===========================================================================
// JAVA — Maven local repository + sources jar walker
// ===========================================================================

/// Discover all external Java dependency roots for a project.
///
/// Strategy:
/// 1. Parse every `pom.xml` under the project root via the existing Maven
///    manifest reader — returns full `MavenCoord` triples (groupId,
///    artifactId, version).
/// 2. Locate the Maven local repository in this order:
///    - `BEARWISDOM_JAVA_MAVEN_REPO` env override
///    - `$HOME/.m2/repository` (or `%USERPROFILE%\.m2\repository` on Windows)
/// 3. For each coord, compute the artifact directory
///    `{repo}/{groupId.replace('.', '/')}/{artifactId}/{version}` and look
///    for the sources jar `{artifactId}-{version}-sources.jar` inside it.
///    When the pom didn't specify a version, scan the artifact directory
///    and pick the lexicographically latest subdirectory as the version.
/// 4. Extract the jar's `.java` entries to a persistent cache directory
///    `{repo}/../bearwisdom-sources-cache/{coord_slug}/` so re-indexing
///    stays fast. Returns one `ExternalDepRoot` per dep pointing at the
///    cache directory.
///
/// Test jars (`-test-sources.jar`) and classifier-suffixed variants are
/// skipped intentionally — they aren't part of the public API surface and
/// would inflate the symbol table with test scaffolding.
pub fn discover_java_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    use crate::indexer::manifest::maven::{parse_pom_xml_coords, MavenCoord};

    // Collect every pom.xml under the project. Reusing the MavenManifest
    // walker would only surface groupIds, so we re-walk here for the full
    // coordinates.
    let mut pom_paths: Vec<PathBuf> = Vec::new();
    collect_pom_files_bounded(project_root, &mut pom_paths, 0);
    if pom_paths.is_empty() {
        return Vec::new();
    }

    let mut coords: Vec<MavenCoord> = Vec::new();
    for pom in &pom_paths {
        let Ok(content) = std::fs::read_to_string(pom) else {
            continue;
        };
        coords.extend(parse_pom_xml_coords(&content));
    }
    if coords.is_empty() {
        return Vec::new();
    }

    let Some(repo) = maven_local_repo() else {
        debug!("No Maven local repository discovered; skipping Java externals");
        return Vec::new();
    };
    let cache_base = repo.parent().unwrap_or(&repo).join("bearwisdom-sources-cache");
    let _ = std::fs::create_dir_all(&cache_base);

    debug!(
        "Probing Maven local repo {} for {} declared coords",
        repo.display(),
        coords.len()
    );

    let mut roots = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

    for coord in coords {
        let Some((version, artifact_dir)) = resolve_maven_artifact_dir(&repo, &coord) else {
            continue;
        };
        let sources_jar = artifact_dir.join(format!(
            "{}-{}-sources.jar",
            coord.artifact_id, version
        ));
        if !sources_jar.is_file() {
            debug!(
                "Maven sources jar missing for {}:{}:{} — skipping",
                coord.group_id, coord.artifact_id, version
            );
            continue;
        }

        let cache_dir = cache_base
            .join(coord.group_id.replace('.', "_"))
            .join(&coord.artifact_id)
            .join(&version);
        if !cache_dir.exists() || is_cache_stale(&sources_jar, &cache_dir) {
            if let Err(e) = extract_java_sources_jar(&sources_jar, &cache_dir) {
                debug!(
                    "Failed to extract {}: {e}",
                    sources_jar.display()
                );
                continue;
            }
        }

        if !seen.insert(cache_dir.clone()) {
            continue;
        }
        roots.push(ExternalDepRoot {
            module_path: format!("{}:{}", coord.group_id, coord.artifact_id),
            version,
            root: cache_dir,
            ecosystem: "java",
        });
    }
    roots
}

/// Locate `$MAVEN_LOCAL_REPO` in the order BEARWISDOM_JAVA_MAVEN_REPO →
/// `$HOME/.m2/repository` → `$USERPROFILE/.m2/repository`. Returns `None`
/// when no directory is found — Java externals silently drop.
pub fn maven_local_repo() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("BEARWISDOM_JAVA_MAVEN_REPO") {
        let p = PathBuf::from(explicit);
        if p.is_dir() {
            return Some(p);
        }
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let candidate = PathBuf::from(home).join(".m2").join("repository");
    if candidate.is_dir() {
        Some(candidate)
    } else {
        None
    }
}

/// Resolve `{repo}/{groupId/as/path}/{artifactId}/{version}/` for a coord.
/// When `coord.version` is None, fall back to the lexicographically largest
/// subdirectory of `{repo}/{group}/{artifact}/` so Spring Boot starters
/// that resolve `${spring.version}` still match whatever is locally cached.
/// Returns `(resolved_version, artifact_dir)`.
fn resolve_maven_artifact_dir(
    repo: &Path,
    coord: &crate::indexer::manifest::maven::MavenCoord,
) -> Option<(String, PathBuf)> {
    let mut group_path = repo.to_path_buf();
    for seg in coord.group_id.split('.') {
        group_path.push(seg);
    }
    group_path.push(&coord.artifact_id);
    if !group_path.is_dir() {
        return None;
    }

    let version = if let Some(v) = &coord.version {
        v.clone()
    } else {
        // Pick the lexicographically largest subdirectory — not perfect
        // semver ordering but good enough to find any cached version.
        let entries = std::fs::read_dir(&group_path).ok()?;
        let mut versions: Vec<String> = entries
            .flatten()
            .filter_map(|e| {
                if e.file_type().ok()?.is_dir() {
                    e.file_name().into_string().ok()
                } else {
                    None
                }
            })
            .collect();
        versions.sort();
        versions.into_iter().next_back()?
    };

    let artifact_dir = group_path.join(&version);
    if artifact_dir.is_dir() {
        Some((version, artifact_dir))
    } else {
        None
    }
}

/// Mini walker that finds every `pom.xml` under a project root up to a
/// bounded depth. Mirrors the helper in `manifest/maven.rs` because that
/// one is private to the module.
fn collect_pom_files_bounded(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 6 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if let Ok(ft) = entry.file_type() {
            if ft.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if matches!(
                        name,
                        ".git" | "target" | "build" | "node_modules"
                            | ".gradle" | "bin" | "obj" | ".idea"
                    ) {
                        continue;
                    }
                }
                collect_pom_files_bounded(&path, out, depth + 1);
            } else if ft.is_file() {
                if path.file_name().and_then(|n| n.to_str()) == Some("pom.xml") {
                    out.push(path);
                }
            }
        }
    }
}

/// Compare the sources jar mtime against the newest `.java` file under
/// `cache_dir`. If the jar was updated more recently, the cache is stale
/// and callers should re-extract.
fn is_cache_stale(jar: &Path, cache_dir: &Path) -> bool {
    let jar_mtime = match std::fs::metadata(jar).and_then(|m| m.modified()) {
        Ok(t) => t,
        Err(_) => return true,
    };
    let entries = match std::fs::read_dir(cache_dir) {
        Ok(e) => e,
        Err(_) => return true,
    };
    let mut newest: Option<std::time::SystemTime> = None;
    for entry in entries.flatten() {
        if let Ok(md) = entry.metadata() {
            if let Ok(t) = md.modified() {
                newest = Some(newest.map(|cur| cur.max(t)).unwrap_or(t));
            }
        }
    }
    match newest {
        Some(t) => jar_mtime > t,
        None => true,
    }
}

/// Extract all `.java` entries from a Maven `-sources.jar` into `dest`.
/// Skips entries whose path traverses out of `dest` (zip-slip guard) and
/// ignores non-`.java` files (META-INF, pom.properties, etc.).
fn extract_java_sources_jar(jar_path: &Path, dest: &Path) -> std::io::Result<()> {
    use std::io::{Read, Write};

    std::fs::create_dir_all(dest)?;
    let file = std::fs::File::open(jar_path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        if entry.is_dir() {
            continue;
        }
        let Some(entry_path) = entry.enclosed_name() else {
            continue;
        };
        let entry_path = entry_path.to_path_buf();
        let Some(name) = entry_path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.ends_with(".java") {
            continue;
        }
        let out_path = dest.join(&entry_path);
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out_file = std::fs::File::create(&out_path)?;
        let mut buf = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut buf)?;
        out_file.write_all(&buf)?;
    }
    Ok(())
}

/// Walk one Java external dep root and emit `WalkedFile` entries.
///
/// File filtering rules:
/// - Only `.java` files.
/// - Skip `package-info.java` (package-level annotations only) and
///   `module-info.java` (JPMS module descriptor, not public API).
/// - Skip `tests/`, `test/`, `*Test.java`, `*Tests.java`.
///
/// Virtual relative_path is `ext:java:{groupId:artifactId}/{sub_path}`.
pub fn walk_java_external_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_java_dir(&dep.root, &dep.root, dep, &mut out);
    out
}

fn walk_java_dir(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "test" | "tests" | "META-INF") {
                    continue;
                }
            }
            walk_java_dir(&path, root, dep, out);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !name.ends_with(".java") {
                continue;
            }
            if name == "package-info.java" || name == "module-info.java" {
                continue;
            }
            if name.ends_with("Test.java") || name.ends_with("Tests.java") {
                continue;
            }

            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            let virtual_path = format!("ext:java:{}/{}", dep.module_path, rel_sub);

            out.push(WalkedFile {
                relative_path: virtual_path,
                absolute_path: path,
                language: "java",
            });
        }
    }
}

// ===========================================================================
// .NET — NuGet global packages cache + DLL metadata reader
// ===========================================================================

/// A parsed .NET external source: a synthetic `ParsedFile` built from
/// a DLL's ECMA-335 metadata, ready to merge into the index.
///
/// Unlike Go/Python/TS/Java, .NET externals don't walk source files.
/// DLLs carry metadata but no source. `parse_dotnet_externals` uses
/// `dotscope` to enumerate types + methods directly and emits one
/// `ParsedFile` per DLL with one `ExtractedSymbol` per type/method.
///
/// The returned files have:
/// - `path`   : `ext:dotnet:{package_id}/{tfm}/{assembly_name}`
/// - `language`: `csharp` (so CLI search still matches by language filter)
/// - `symbols`: class/interface/struct/enum symbols from `types()`,
///              plus method symbols with `qualified_name = namespace.type.method`
pub fn parse_dotnet_externals(project_root: &Path) -> Vec<crate::types::ParsedFile> {
    use crate::indexer::manifest::nuget::parse_package_references_full;

    // Walk the project for .csproj / .fsproj / .vbproj and collect coords.
    let mut project_files: Vec<PathBuf> = Vec::new();
    collect_dotnet_project_files(project_root, &mut project_files, 0);
    if project_files.is_empty() {
        return Vec::new();
    }

    let mut coords: Vec<crate::indexer::manifest::nuget::NuGetCoord> = Vec::new();
    for p in &project_files {
        let Ok(content) = std::fs::read_to_string(p) else {
            continue;
        };
        coords.extend(parse_package_references_full(&content));
    }

    // Augment with transitive dependencies from `.deps.json`. The dotnet
    // SDK emits one per project under bin/{config}/{tfm}/{project}.deps.json
    // after `dotnet build`. It enumerates every assembly loaded at runtime,
    // including transitives that `.csproj` only declares indirectly
    // (`Microsoft.Extensions.Hosting` pulls in 30+ packages). This augments
    // the direct list without walking the whole NuGet cache.
    //
    // De-dup happens later at the dll_path level — reading the same package
    // declared as both a direct dep and a transitive is cheap because the
    // `seen` set in the main loop catches it.
    for p in &project_files {
        if let Some(proj_dir) = p.parent() {
            coords.extend(collect_transitive_coords_from_deps_json(proj_dir));
        }
    }

    if coords.is_empty() {
        return Vec::new();
    }

    let Some(nuget_root) = nuget_packages_root() else {
        debug!("No NuGet packages cache discovered; skipping .NET externals");
        return Vec::new();
    };

    debug!(
        "Probing NuGet cache {} for {} package references",
        nuget_root.display(),
        coords.len()
    );

    // Language tag from the project file type: VB and F# call sites
    // still see .NET metadata through the same DLL, but CLI language
    // filters and per-language stats should attribute the symbols to
    // the caller's source language.
    let lang_id = dominant_dotnet_language(&project_files);

    let mut out = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for coord in coords {
        let Some(dll_path) = resolve_nuget_dll(&nuget_root, &coord) else {
            continue;
        };
        if !seen.insert(dll_path.clone()) {
            continue;
        }
        match parse_dotnet_dll(&dll_path, &coord.name, lang_id) {
            Ok(pf) => out.push(pf),
            Err(e) => debug!(
                "Failed to read .NET metadata from {}: {e}",
                dll_path.display()
            ),
        }
    }
    out
}

/// Collect transitive NuGet dependencies by reading `.deps.json` files
/// emitted under `{proj_dir}/bin/{config}/{tfm}/`. Each runtime library
/// listed with `"type": "package"` becomes a `NuGetCoord` so the main
/// externals pass can resolve its DLL in the global packages cache.
///
/// Returns an empty vector when no build output exists — that's the
/// expected state on a fresh checkout and the direct-dep pass in the
/// caller handles the common case fine. The transitive augmentation
/// only kicks in when the user has actually built their project at
/// least once.
///
/// Scans at most 16 deps.json files per project to avoid pathological
/// matrix TFM builds inflating the coord list. In the overwhelmingly
/// common single-TFM case this cap is irrelevant.
fn collect_transitive_coords_from_deps_json(
    proj_dir: &Path,
) -> Vec<crate::indexer::manifest::nuget::NuGetCoord> {
    let mut deps_json_files: Vec<PathBuf> = Vec::new();
    collect_deps_json(&proj_dir.join("bin"), &mut deps_json_files, 0);
    if deps_json_files.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for path in deps_json_files.iter().take(16) {
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) else {
            continue;
        };
        // The `libraries` map is keyed by `{name}/{version}` and each
        // entry carries a `type` field. We want `type == "package"`
        // entries — local projects (`type == "project"`) and reference
        // assemblies (`type == "referenceassembly"`) aren't NuGet-cached.
        let Some(libs) = json.get("libraries").and_then(|v| v.as_object()) else {
            continue;
        };
        for (key, value) in libs {
            let ty = value
                .get("type")
                .and_then(|t| t.as_str())
                .unwrap_or("");
            if ty != "package" {
                continue;
            }
            let Some((name, version)) = key.rsplit_once('/') else {
                continue;
            };
            if !seen.insert(key.clone()) {
                continue;
            }
            out.push(crate::indexer::manifest::nuget::NuGetCoord {
                name: name.to_string(),
                version: Some(version.to_string()),
            });
        }
    }
    out
}

/// Walk a `bin/` tree collecting every `*.deps.json` file. Bounded
/// depth to avoid accidental traversal outside the build output. Skips
/// `obj/` and `runtimes/` to stay focused on the actual TFM outputs.
fn collect_deps_json(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 5 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if let Ok(ft) = entry.file_type() {
            if ft.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if matches!(name, "obj" | "runtimes" | "ref") {
                        continue;
                    }
                }
                collect_deps_json(&path, out, depth + 1);
            } else if ft.is_file() {
                if path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with(".deps.json"))
                {
                    out.push(path);
                }
            }
        }
    }
}

/// Determine the language tag to attribute external .NET symbols to.
/// Scans the project files found in the consumer tree and picks the
/// most common extension: `csharp` for `.csproj`, `fsharp` for `.fsproj`,
/// `vb` for `.vbproj`. If the project is a mix, C# wins — it's by far
/// the most common language and downstream search defaults to it.
fn dominant_dotnet_language(project_files: &[PathBuf]) -> &'static str {
    let mut cs = 0usize;
    let mut fs = 0usize;
    let mut vb = 0usize;
    for p in project_files {
        match p.extension().and_then(|e| e.to_str()) {
            Some("csproj") => cs += 1,
            Some("fsproj") => fs += 1,
            Some("vbproj") => vb += 1,
            _ => {}
        }
    }
    // C# is the default tiebreaker — it's the overwhelming majority on
    // NuGet and in the .NET ecosystem at large.
    if cs >= fs && cs >= vb {
        "csharp"
    } else if fs >= vb {
        "fsharp"
    } else {
        "vb"
    }
}

/// Locate the NuGet global packages folder in this order:
/// `BEARWISDOM_NUGET_PACKAGES` env override → `NUGET_PACKAGES` env →
/// `$HOME/.nuget/packages` (or `%USERPROFILE%\.nuget\packages` on Windows).
pub fn nuget_packages_root() -> Option<PathBuf> {
    for key in ["BEARWISDOM_NUGET_PACKAGES", "NUGET_PACKAGES"] {
        if let Some(raw) = std::env::var_os(key) {
            let p = PathBuf::from(raw);
            if p.is_dir() {
                return Some(p);
            }
        }
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let candidate = PathBuf::from(home).join(".nuget").join("packages");
    if candidate.is_dir() {
        Some(candidate)
    } else {
        None
    }
}

/// Resolve `{nuget_root}/{pkg}/{version}/lib/{tfm}/{pkg}.dll` for a coord.
///
/// The NuGet package folder is lowercase on disk. Inside, version dirs are
/// the concrete version strings; we prefer the caller's declared version
/// but fall back to the lexicographically largest when it's missing or
/// when the declared version isn't on disk.
///
/// Inside `lib/`, there may be multiple target frameworks. We prefer in
/// order: `net9.0`, `net8.0`, `net7.0`, `net6.0`, `netstandard2.1`,
/// `netstandard2.0` — newer frameworks tend to have more surface area.
/// If none of these are present, fall back to the lexicographically
/// largest subdirectory.
fn resolve_nuget_dll(
    nuget_root: &Path,
    coord: &crate::indexer::manifest::nuget::NuGetCoord,
) -> Option<PathBuf> {
    let pkg_dir = nuget_root.join(coord.name.to_lowercase());
    if !pkg_dir.is_dir() {
        return None;
    }

    let version = if let Some(v) = &coord.version {
        let concrete = pkg_dir.join(v);
        if concrete.is_dir() {
            v.clone()
        } else {
            largest_version_subdir(&pkg_dir)?
        }
    } else {
        largest_version_subdir(&pkg_dir)?
    };

    let version_dir = pkg_dir.join(&version);
    let lib_dir = version_dir.join("lib");
    if !lib_dir.is_dir() {
        return None;
    }

    let preferred_tfms = [
        "net9.0",
        "net8.0",
        "net7.0",
        "net6.0",
        "netstandard2.1",
        "netstandard2.0",
    ];
    let mut chosen_tfm: Option<PathBuf> = None;
    for tfm in preferred_tfms {
        let candidate = lib_dir.join(tfm);
        if candidate.is_dir() {
            chosen_tfm = Some(candidate);
            break;
        }
    }
    let tfm_dir = chosen_tfm.or_else(|| largest_subdir(&lib_dir))?;

    // The DLL filename matches the package name (case-insensitive). Scan
    // for a `.dll` that matches instead of guessing exact case.
    let entries = std::fs::read_dir(&tfm_dir).ok()?;
    let target_lower = coord.name.to_lowercase() + ".dll";
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if name == target_lower {
            return Some(entry.path());
        }
    }
    None
}

/// Pick the lexicographically largest subdirectory name — a crude stand-in
/// for semver ordering that's good enough for finding any cached version.
fn largest_version_subdir(dir: &Path) -> Option<String> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut versions: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            if e.file_type().ok()?.is_dir() {
                e.file_name().into_string().ok()
            } else {
                None
            }
        })
        .collect();
    versions.sort();
    versions.into_iter().next_back()
}

fn largest_subdir(dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut subs: Vec<PathBuf> = entries
        .flatten()
        .filter_map(|e| {
            if e.file_type().ok()?.is_dir() {
                Some(e.path())
            } else {
                None
            }
        })
        .collect();
    subs.sort();
    subs.into_iter().next_back()
}

/// Parse a single .NET DLL and emit a synthetic `ParsedFile` with one
/// symbol per type (`Class` / `Interface` / `Struct` / `Enum`) and one
/// symbol per method. Signatures include the type's generic parameters
/// and the method's parameter/return types, with ECMA-335 placeholder
/// indices (`!0`, `!!0`) substituted back to the real parameter names
/// from the GenericParam metadata tables so the resolver's generic-param
/// classifier fires for C# externals the same way it does for TS after S6.
///
/// Per-type method iteration: methods are read via `type_def.methods`
/// (weak refs upgraded lazily) rather than the global `assembly.methods()`
/// + `declaring_type_fullname()` lookup. This gives direct attribution
/// without the per-method fullname formatting work that S7 paid.
///
/// Public surface only — types with non-public visibility and methods
/// with non-public visibility are skipped. Compiler-generated types
/// (`<>c`, `<PrivateImplementationDetails>`, `<Module>`) are filtered to
/// avoid polluting the index with noise no user code can reference.
///
/// The `lang_id` caller-chosen language tag is propagated onto the
/// synthetic `ParsedFile`; callers pick it based on whether the owning
/// project was a .csproj (`csharp`), .fsproj (`fsharp`), or .vbproj
/// (`vb`). The DLL itself is the same metadata format regardless — only
/// the display language differs.
fn parse_dotnet_dll(
    dll_path: &Path,
    package_name: &str,
    lang_id: &str,
) -> std::result::Result<crate::types::ParsedFile, String> {
    use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind};
    use dotscope::metadata::method::MethodAccessFlags;
    use dotscope::prelude::CilObject;

    let assembly = CilObject::from_path(dll_path).map_err(|e| e.to_string())?;

    let assembly_name = assembly
        .assembly()
        .map(|a| a.name.clone())
        .unwrap_or_else(|| package_name.to_string());

    let virtual_path = format!("ext:dotnet:{}/{}", package_name, assembly_name);
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();

    for type_def in assembly.types().all_types().iter() {
        let name = type_def.name.clone();
        let namespace = type_def.namespace.clone();

        // Skip compiler-generated types. These have names like `<>c`,
        // `<PrivateImplementationDetails>`, `<Module>` and inflate the
        // symbol table with noise no user code can reference.
        if name.starts_with('<') || name == "<Module>" {
            continue;
        }

        // Skip non-public types — public API surface only.
        // TypeAttributes.VisibilityMask = 0x07
        let visibility_mask = type_def.flags & 0x07;
        if visibility_mask != 1 && visibility_mask != 2 {
            // 1 = Public, 2 = NestedPublic; everything else is private/internal.
            continue;
        }

        // Interface flag = TypeAttributes.ClassSemanticsMask & 0x20
        let is_interface = type_def.flags & 0x20 != 0;
        let kind = if is_interface {
            SymbolKind::Interface
        } else {
            SymbolKind::Class
        };

        // Strip the ECMA-335 backtick-arity suffix (`Repository\`1` → `Repository`)
        // so user code that references `Repository<User>` resolves to the
        // right symbol. The arity is reflected in the generic_params vec.
        let display_name = strip_backtick_arity(&name);
        let qualified_name = if namespace.is_empty() {
            display_name.to_string()
        } else {
            format!("{namespace}.{display_name}")
        };

        // Build the real `<T, U>` suffix from the GenericParam table
        // rather than making up `<T1, T2, ...>` from the backtick count.
        let type_generic_names: Vec<String> = type_def
            .generic_params
            .iter()
            .map(|(_, gp)| gp.name.clone())
            .collect();
        let type_gp_suffix = format_generic_suffix(&type_generic_names);

        symbols.push(ExtractedSymbol {
            name: display_name.to_string(),
            qualified_name: qualified_name.clone(),
            kind,
            visibility: Some(crate::types::Visibility::Public),
            start_line: 0,
            end_line: 0,
            start_col: 0,
            end_col: 0,
            signature: Some(format!(
                "{} {}{}",
                if is_interface { "interface" } else { "class" },
                display_name,
                type_gp_suffix
            )),
            doc_comment: None,
            scope_path: if namespace.is_empty() {
                None
            } else {
                Some(namespace.clone())
            },
            parent_index: None,
        });

        // Per-type method iteration: walk type_def.methods directly so we
        // get method-to-type attribution for free and avoid a second pass
        // over the global method map. `boxcar::Vec` yields `(usize, &T)`
        // tuples; we only care about the ref.
        for (_, method_ref) in type_def.methods.iter() {
            let Some(method) = method_ref.upgrade() else {
                continue;
            };

            // Skip compiler-generated accessors and lifecycle methods:
            // - `get_X` / `set_X` / `add_X` / `remove_X` (property/event accessors)
            // - `.ctor` / `.cctor` (constructors emit as Constructor symbols elsewhere)
            // - `<...>` anonymous/closure methods
            if method.name.starts_with('<') || method.name.starts_with('.') {
                continue;
            }
            // Public surface only.
            if method.flags_access != MethodAccessFlags::PUBLIC {
                continue;
            }

            let method_name = method.name.clone();
            let method_qname = format!("{qualified_name}.{method_name}");

            // Collect the method's own generic param names so we can
            // splice them into the signature and substitute `!!N`
            // placeholders back to real names.
            let method_generic_names: Vec<String> = method
                .generic_params
                .iter()
                .map(|(_, gp)| gp.name.clone())
                .collect();

            let signature = format_method_signature(
                &method_name,
                &method.signature,
                &type_generic_names,
                &method_generic_names,
                &assembly,
            );

            symbols.push(ExtractedSymbol {
                name: method_name,
                qualified_name: method_qname,
                kind: SymbolKind::Method,
                visibility: Some(crate::types::Visibility::Public),
                start_line: 0,
                end_line: 0,
                start_col: 0,
                end_col: 0,
                signature: Some(signature),
                doc_comment: None,
                scope_path: Some(qualified_name.clone()),
                parent_index: None,
            });
        }
    }

    let symbol_count = symbols.len();
    debug!(
        "Parsed {} external .NET symbols from {}",
        symbol_count,
        dll_path.display()
    );

    // Compute a content hash from the DLL bytes so incremental indexing
    // knows when to re-read. Use the file mtime + size as a cheap proxy
    // rather than hashing the whole DLL every time.
    let metadata = std::fs::metadata(dll_path).map_err(|e| e.to_string())?;
    let size = metadata.len();
    let mtime = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64);
    let content_hash = format!("{:x}", size).to_string();

    Ok(ParsedFile {
        path: virtual_path,
        language: lang_id.to_string(),
        content_hash,
        size,
        line_count: 0,
        mtime,
        package_id: None,
        symbols,
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        content: None,
        has_errors: false,
    })
}

/// Strip the ECMA-335 backtick arity suffix from a type name.
///
/// `Repository\`1` → `Repository`, `Dictionary\`2` → `Dictionary`,
/// `Func\`4` → `Func`. Left-idempotent on names without a backtick.
/// This is the surface name users write in source — the arity is
/// reflected separately in the `generic_params` collection.
fn strip_backtick_arity(name: &str) -> &str {
    match name.find('`') {
        Some(idx) => &name[..idx],
        None => name,
    }
}

/// Format a list of generic parameter names as `<A, B, C>` or empty if
/// the list is empty. Kept as a helper so the type and method signature
/// builders stay readable.
fn format_generic_suffix(names: &[String]) -> String {
    if names.is_empty() {
        String::new()
    } else {
        format!("<{}>", names.join(", "))
    }
}

/// Format a method signature in a shape the resolver's generic-param
/// classifier and chain walker can parse. The classifier scans for
/// `<...>` at the top level of a signature string and splits on commas
/// to extract parameter names; the chain walker reads the return type
/// portion after the `:` separator.
///
/// Shape: `{method_name}<U, V>(Param1, Param2): ReturnType`
///
/// Parameter and return type strings get two post-processing passes:
/// 1. ECMA-335 placeholder substitution (`!N` → type param, `!!N` → method param)
/// 2. Metadata-token resolution: `class[00000042]` and `valuetype[00000042]`
///    → `Namespace.TypeName` via a `TypeRegistry` lookup.
///
/// Nested `GenericInst(class[…], args)` becomes `TypeName<T, U>` in one
/// pass — Display renders `class[…]<T, U>` and the token substitution
/// rewrites the leading `class[…]` to `TypeName` without touching the
/// already-valid generic argument list.
fn format_method_signature(
    method_name: &str,
    sig: &dotscope::metadata::signatures::SignatureMethod,
    type_generic_names: &[String],
    method_generic_names: &[String],
    assembly: &dotscope::prelude::CilObject,
) -> String {
    let gp_suffix = format_generic_suffix(method_generic_names);

    let mut params_str = String::from("(");
    for (i, p) in sig.params.iter().enumerate() {
        if i > 0 {
            params_str.push_str(", ");
        }
        let rendered = format!("{}", p);
        let substituted = substitute_generic_placeholders(
            &rendered,
            type_generic_names,
            method_generic_names,
        );
        params_str.push_str(&resolve_signature_tokens(&substituted, assembly));
    }
    params_str.push(')');

    let return_rendered = format!("{}", sig.return_type);
    let return_substituted = substitute_generic_placeholders(
        &return_rendered,
        type_generic_names,
        method_generic_names,
    );
    let return_str = resolve_signature_tokens(&return_substituted, assembly);

    format!("{method_name}{gp_suffix}{params_str}: {return_str}")
}

/// Replace ECMA-335 `class[HHHHHHHH]` / `valuetype[HHHHHHHH]` token
/// placeholders with their resolved `Namespace.TypeName`. Tries both
/// metadata-table sources:
///
/// - **TypeDef** (token high byte `0x02`): defined in the current
///   assembly, looked up via `assembly.types()` (a `TypeRegistry`).
/// - **TypeRef** (token high byte `0x01`): references to types in
///   other assemblies (`System.String`, `System.Threading.Tasks.Task`,
///   `Microsoft.Extensions.Logging.ILogger`, etc.), looked up via
///   `assembly.imports()`. Most nested type arguments in real .NET
///   signatures fall into this bucket — they reference types defined
///   in the BCL or other dependency assemblies.
///
/// Leaves unresolvable tokens as-is so the signature still renders and
/// the top-level `<...>` region stays parseable by downstream code.
/// `dotscope`'s `TypeSignature::Display` emits tokens as upper-case
/// 8-hex-digit values wrapped in square brackets; we scan for both
/// prefixes, parse the hex, select the right lookup via the token's
/// high byte, and splice the result back in.
fn resolve_signature_tokens(
    rendered: &str,
    assembly: &dotscope::prelude::CilObject,
) -> String {
    use dotscope::metadata::token::Token;

    let type_registry = assembly.types();
    let imports = assembly.imports().cil();

    let mut out = String::with_capacity(rendered.len());
    let bytes = rendered.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let remaining = &rendered[i..];
        let (prefix_len, skip_prefix) = if remaining.starts_with("class[") {
            (6, true)
        } else if remaining.starts_with("valuetype[") {
            (10, true)
        } else {
            (0, false)
        };
        if skip_prefix {
            // Scan for closing bracket.
            let after_prefix = &remaining[prefix_len..];
            if let Some(close_rel) = after_prefix.find(']') {
                let hex = &after_prefix[..close_rel];
                if let Ok(value) = u32::from_str_radix(hex, 16) {
                    let token = Token::new(value);
                    // High byte selects the metadata table:
                    //   0x02 = TypeDef  (current assembly)
                    //   0x01 = TypeRef  (external assemblies — BCL etc.)
                    //   0x1B = TypeSpec (generic instantiations, not handled here)
                    let table_byte = value >> 24;
                    let resolved: Option<String> = match table_byte {
                        0x02 => type_registry.get(&token).map(|ty| {
                            let name = strip_backtick_arity(&ty.name).to_string();
                            if ty.namespace.is_empty() {
                                name
                            } else {
                                format!("{}.{}", ty.namespace, name)
                            }
                        }),
                        0x01 => imports.get(token).map(|imp| {
                            let name = strip_backtick_arity(&imp.name).to_string();
                            if imp.namespace.is_empty() {
                                name
                            } else {
                                format!("{}.{}", imp.namespace, name)
                            }
                        }),
                        _ => None,
                    };
                    if let Some(full) = resolved {
                        out.push_str(&full);
                        i += prefix_len + close_rel + 1;
                        continue;
                    }
                }
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Replace ECMA-335 generic parameter placeholders with their real names.
///
/// `!0` → first type generic parameter (e.g., `T`)
/// `!!0` → first method generic parameter (e.g., `U`)
///
/// Scans left-to-right, handling multi-digit indices (`!10`, `!!10`).
/// Unknown indices are left as-is so the signature still renders but
/// unrecognised generic params don't crash the formatter. Method-level
/// `!!N` must be checked BEFORE type-level `!N` because `!!` would
/// otherwise be consumed as two separate `!0` matches.
fn substitute_generic_placeholders(
    rendered: &str,
    type_gen: &[String],
    method_gen: &[String],
) -> String {
    let bytes = rendered.as_bytes();
    let mut out = String::with_capacity(rendered.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'!' {
            let is_method = i + 1 < bytes.len() && bytes[i + 1] == b'!';
            let num_start = if is_method { i + 2 } else { i + 1 };
            let mut num_end = num_start;
            while num_end < bytes.len() && bytes[num_end].is_ascii_digit() {
                num_end += 1;
            }
            if num_end > num_start {
                let idx: usize = rendered[num_start..num_end].parse().unwrap_or(usize::MAX);
                let target = if is_method { method_gen } else { type_gen };
                if let Some(name) = target.get(idx) {
                    out.push_str(name);
                    i = num_end;
                    continue;
                }
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn collect_dotnet_project_files(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 10 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if let Ok(ft) = entry.file_type() {
            if ft.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if matches!(
                        name,
                        "bin" | "obj" | "node_modules" | ".git" | "target"
                            | "packages" | ".vs" | "TestResults" | "artifacts"
                    ) {
                        continue;
                    }
                }
                collect_dotnet_project_files(&path, out, depth + 1);
            } else if ft.is_file() {
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if matches!(ext, "csproj" | "fsproj" | "vbproj") {
                        out.push(path);
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_preserves_lowercase_paths() {
        assert_eq!(
            escape_module_path("github.com/gin-gonic/gin"),
            "github.com/gin-gonic/gin"
        );
    }

    #[test]
    fn escape_handles_uppercase_segments() {
        assert_eq!(
            escape_module_path("github.com/Microsoft/go-winio"),
            "github.com/!microsoft/go-winio"
        );
        assert_eq!(
            escape_module_path("github.com/AlecAivazis/survey"),
            "github.com/!alec!aivazis/survey"
        );
    }

    #[test]
    fn discover_returns_empty_without_go_mod() {
        let tmp = std::env::temp_dir().join("bw-test-no-go-mod");
        let _ = std::fs::create_dir_all(&tmp);
        let result = discover_go_externals(&tmp);
        assert!(result.is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn strip_backtick_arity_removes_generic_suffix() {
        assert_eq!(strip_backtick_arity("Repository`1"), "Repository");
        assert_eq!(strip_backtick_arity("Dictionary`2"), "Dictionary");
        assert_eq!(strip_backtick_arity("Func`4"), "Func");
        assert_eq!(strip_backtick_arity("List"), "List");
        assert_eq!(strip_backtick_arity(""), "");
    }

    #[test]
    fn format_generic_suffix_joins_names() {
        assert_eq!(format_generic_suffix(&[]), "");
        assert_eq!(
            format_generic_suffix(&["T".to_string()]),
            "<T>"
        );
        assert_eq!(
            format_generic_suffix(&["T".to_string(), "U".to_string()]),
            "<T, U>"
        );
    }

    #[test]
    fn substitute_placeholders_swaps_ecma335_syntax() {
        let type_gen = vec!["T".to_string()];
        let method_gen = vec!["U".to_string(), "V".to_string()];

        // Method-level placeholder.
        assert_eq!(
            substitute_generic_placeholders("!!0", &type_gen, &method_gen),
            "U"
        );
        assert_eq!(
            substitute_generic_placeholders("!!1", &type_gen, &method_gen),
            "V"
        );
        // Type-level placeholder.
        assert_eq!(
            substitute_generic_placeholders("!0", &type_gen, &method_gen),
            "T"
        );
        // Mixed inside a call-site signature.
        assert_eq!(
            substitute_generic_placeholders(
                "Func<!0, !!0, !!1>",
                &type_gen,
                &method_gen
            ),
            "Func<T, U, V>"
        );
        // Out-of-range index is left alone.
        assert_eq!(
            substitute_generic_placeholders("!!5", &type_gen, &method_gen),
            "!!5"
        );
    }

    #[test]
    fn substitute_placeholders_multi_digit_indices() {
        let method_gen: Vec<String> = (0..15).map(|i| format!("T{i}")).collect();
        assert_eq!(
            substitute_generic_placeholders("!!10", &[], &method_gen),
            "T10"
        );
        assert_eq!(
            substitute_generic_placeholders("!!14", &[], &method_gen),
            "T14"
        );
    }

    #[test]
    fn python_name_normalization_strips_extras_and_versions() {
        assert_eq!(normalize_python_dep_name("fastapi"), "fastapi");
        assert_eq!(
            normalize_python_dep_name("fastapi[standard]<1.0.0,>=0.114.2"),
            "fastapi"
        );
        assert_eq!(
            normalize_python_dep_name("pydantic-settings>=2.2.1"),
            "pydantic_settings"
        );
        assert_eq!(
            normalize_python_dep_name("SQLAlchemy>=2.0"),
            "sqlalchemy"
        );
        assert_eq!(
            normalize_python_dep_name("psycopg[binary]<4.0.0,>=3.1.13"),
            "psycopg"
        );
    }

    #[test]
    fn python_name_normalization_handles_environment_markers() {
        assert_eq!(
            normalize_python_dep_name("urllib3<2;python_version<'3.10'"),
            "urllib3"
        );
        assert_eq!(
            normalize_python_dep_name("some-pkg @ git+https://github.com/x/y"),
            "some_pkg"
        );
    }

    #[test]
    fn definitely_typed_scoped_escapes() {
        assert_eq!(
            definitely_typed_scoped_name("@tanstack/react-query"),
            Some("tanstack__react-query".to_string())
        );
        assert_eq!(
            definitely_typed_scoped_name("@radix-ui/react-dialog"),
            Some("radix-ui__react-dialog".to_string())
        );
        assert_eq!(definitely_typed_scoped_name("react"), None);
        assert_eq!(definitely_typed_scoped_name("@scope"), None);
        assert_eq!(definitely_typed_scoped_name("@/empty"), None);
    }

    #[test]
    fn ts_source_file_detection() {
        assert!(is_ts_source_file("index.ts"));
        assert!(is_ts_source_file("App.tsx"));
        assert!(is_ts_source_file("index.d.ts"));
        assert!(is_ts_source_file("types.d.mts"));
        assert!(!is_ts_source_file("index.js"));
        assert!(!is_ts_source_file("README.md"));
        assert!(!is_ts_source_file("package.json"));
    }

    #[test]
    fn ts_test_file_detection() {
        assert!(is_test_or_story_file("Foo.test.ts"));
        assert!(is_test_or_story_file("Foo.spec.tsx"));
        assert!(is_test_or_story_file("Button.stories.tsx"));
        assert!(is_test_or_story_file("perf.bench.ts"));
        assert!(!is_test_or_story_file("index.ts"));
        assert!(!is_test_or_story_file("App.tsx"));
        assert!(!is_test_or_story_file("useForm.ts"));
    }
}
