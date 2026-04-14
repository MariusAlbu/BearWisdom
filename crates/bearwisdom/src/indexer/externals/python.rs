// Python site-packages discovery + walker

use super::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::indexer::manifest::pyproject::PyProjectManifest;
use crate::indexer::manifest::ManifestReader;
use crate::walker::WalkedFile;
use std::path::{Path, PathBuf};
use tracing::debug;

/// Python site-packages → `discover_python_externals` + `walk_python_external_root`.
pub struct PythonExternalsLocator;

impl ExternalSourceLocator for PythonExternalsLocator {
    fn ecosystem(&self) -> &'static str { "python" }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_python_externals(project_root)
    }

    /// M3: per-package discovery. Reads this package's own pyproject.toml
    /// and probes `{package}/.venv` + every ancestor venv up to
    /// `workspace_root` — covers monorepo layouts with a shared venv at
    /// the root (`/.venv`) alongside per-service venvs (`services/api/.venv`).
    fn locate_roots_for_package(
        &self,
        workspace_root: &Path,
        package_abs_path: &Path,
        package_id: i64,
    ) -> Vec<ExternalDepRoot> {
        let mut roots = discover_python_externals_scoped(workspace_root, package_abs_path);
        for r in &mut roots {
            r.package_id = Some(package_id);
        }
        roots
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_python_external_root(dep)
    }
}

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
                    package_id: None,
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
                    package_id: None,
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
                    package_id: None,
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
                    package_id: None,
                });
            }
        }
        return Some(out);
    }
    None
}

/// M3: per-package variant. Reads only the single package's
/// `pyproject.toml` and probes `{package}/.venv` plus every ancestor
/// venv up to `workspace_root` (inclusive). Returns roots with
/// `package_id=None`; the caller (locator) stamps ownership.
pub fn discover_python_externals_scoped(
    workspace_root: &Path,
    package_abs_path: &Path,
) -> Vec<ExternalDepRoot> {
    let manifest = PyProjectManifest;
    // PyProjectManifest.read walks subdirs; acceptable here since a package
    // directory tree typically only contains its own pyproject.toml.
    let Some(data) = manifest.read(package_abs_path) else {
        return Vec::new();
    };
    if data.dependencies.is_empty() {
        return Vec::new();
    }

    let site_packages = find_python_site_packages_with_ancestors(package_abs_path, workspace_root);
    if site_packages.is_empty() {
        debug!("No Python site-packages discovered for package at {}; skipping Python externals",
            package_abs_path.display());
        return Vec::new();
    }

    let mut roots = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

    for dep_raw in &data.dependencies {
        let normalized = normalize_python_dep_name(dep_raw);
        if normalized.is_empty() {
            continue;
        }
        let mut matched = false;
        for sp in &site_packages {
            let pkg_dir = sp.join(&normalized);
            if pkg_dir.is_dir() && !seen.contains(&pkg_dir) {
                seen.insert(pkg_dir.clone());
                roots.push(ExternalDepRoot {
                    module_path: normalized.clone(),
                    version: String::from("unknown"),
                    root: pkg_dir,
                    ecosystem: "python",
                    package_id: None,
                });
                matched = true;
                break;
            }
            let file = sp.join(format!("{normalized}.py"));
            if file.is_file() && !seen.contains(&file) {
                seen.insert(file.clone());
                roots.push(ExternalDepRoot {
                    module_path: normalized.clone(),
                    version: String::from("unknown"),
                    root: file,
                    ecosystem: "python",
                    package_id: None,
                });
                matched = true;
                break;
            }
        }
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

/// Probe for venv site-packages starting at `start` and walking up to
/// `workspace_root` (inclusive). Respects `BEARWISDOM_PYTHON_SITE_PACKAGES`
/// as an explicit override that short-circuits the filesystem walk.
fn find_python_site_packages_with_ancestors(start: &Path, workspace_root: &Path) -> Vec<PathBuf> {
    if let Some(raw) = std::env::var_os("BEARWISDOM_PYTHON_SITE_PACKAGES") {
        let mut out = Vec::new();
        for seg in std::env::split_paths(&raw) {
            if seg.as_os_str().is_empty() { continue; }
            if seg.is_dir() && !out.contains(&seg) {
                out.push(seg);
            }
        }
        if !out.is_empty() { return out; }
    }

    let mut out: Vec<PathBuf> = Vec::new();
    let mut push_if_dir = |p: PathBuf, out: &mut Vec<PathBuf>| {
        if p.is_dir() && !out.contains(&p) {
            out.push(p);
        }
    };

    // Walk from `start` up to (and including) `workspace_root`, probing
    // venv dirs at every level.
    let mut current: Option<&Path> = Some(start);
    while let Some(dir) = current {
        for venv_name in &[".venv", "venv", ".env"] {
            let venv = dir.join(venv_name);
            if !venv.is_dir() { continue; }
            push_if_dir(venv.join("Lib").join("site-packages"), &mut out);
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
        if dir == workspace_root { break; }
        current = dir.parent();
    }
    out
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
    walk_python_dir_bounded(dir, root, dep, out, 0);
}

fn walk_python_dir_bounded(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= MAX_WALK_DEPTH {
        return;
    }
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
            walk_python_dir_bounded(&path, root, dep, out, depth + 1);
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
