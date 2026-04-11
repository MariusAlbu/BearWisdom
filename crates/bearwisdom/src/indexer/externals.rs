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
/// Indirect deps are skipped for the MVP — they inflate the symbol table
/// for little marginal resolution benefit.
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

    let mut roots = Vec::new();
    for dep in &parsed.require_deps {
        if dep.indirect {
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
                break;
            }
            // Single-file module: site-packages/{normalized}.py
            let file = sp.join(format!("{normalized}.py"));
            if file.is_file() {
                // Wrap the single file as a synthetic root using its parent.
                // The walker will see just this file because we pass a
                // one-file root by putting it directly.
                // Simpler: include the site-packages dir filtered to this one file.
                // We skip single-file modules in MVP to avoid polluting the
                // walker with whole site-packages tree.
                debug!("Single-file module {normalized}.py — skipping (MVP)");
                break;
            }
        }
    }
    roots
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
pub fn walk_python_external_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_python_dir(&dep.root, &dep.root, dep, &mut out);
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
        // 2. The DefinitelyTyped sibling `node_modules/@types/{dep}/` that
        //    carries `.d.ts` for untyped libraries like React, Node, Express.
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
            // DefinitelyTyped fallback only for unscoped packages; scoped
            // packages like `@foo/bar` live under `@types/foo__bar` which
            // we don't try to handle in the MVP.
            if !dep.starts_with('@') {
                let types_dir = nm_root.join("@types").join(dep);
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
                    "node_modules"
                        | "__tests__"
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
