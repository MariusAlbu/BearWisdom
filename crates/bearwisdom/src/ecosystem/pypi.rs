// =============================================================================
// ecosystem/pypi.rs — PyPI ecosystem (Python)
//
// Phase 2 + 3 combined: consolidates the external-source locator
// (`indexer/externals/python.rs`) and the manifest reader
// (`indexer/manifest/pyproject.rs`) into a single ecosystem module.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::ecosystem::manifest::{ManifestData, ManifestKind, ManifestReader};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("pypi");

const MANIFESTS: &[ManifestSpec] = &[];
const LANGUAGES: &[&str] = &["python"];
const LEGACY_ECOSYSTEM_TAG: &str = "python";

pub struct PypiEcosystem;

// ---------------------------------------------------------------------------
// Ecosystem trait impl
// ---------------------------------------------------------------------------

impl Ecosystem for PypiEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Package }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }
    fn manifest_specs(&self) -> &'static [ManifestSpec] { MANIFESTS }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Any(&[
            EcosystemActivation::ManifestMatch,
            EcosystemActivation::LanguagePresent("python"),
        ])
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_python_externals(ctx.project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_python_external_root(dep)
    }

    fn supports_reachability(&self) -> bool { true }

    fn resolve_import(
        &self,
        dep: &ExternalDepRoot,
        _package: &str,
        _symbols: &[&str],
    ) -> Vec<WalkedFile> {
        // Python packages live as either a directory (with __init__.py) or a
        // single-file module (pkg.py). For directory packages, start from
        // __init__.py and follow relative-import expansion bounded at a
        // small depth. Single-file modules return just themselves.
        resolve_python_package_entry(dep)
    }

    fn resolve_symbol(
        &self,
        dep: &ExternalDepRoot,
        _fqn: &str,
    ) -> Vec<WalkedFile> {
        // Same entry point. Re-exports within the package are expanded by
        // resolve_python_package_entry; deeper fqn-specific walking is a
        // later optimization.
        resolve_python_package_entry(dep)
    }
}

// ---------------------------------------------------------------------------
// Legacy ExternalSourceLocator impl
// ---------------------------------------------------------------------------

impl ExternalSourceLocator for PypiEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_python_externals(project_root)
    }

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

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<PypiEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(PypiEcosystem)).clone()
}

// ===========================================================================
// Manifest reader (migrated from indexer/manifest/pyproject.rs)
// ===========================================================================

pub struct PyProjectManifest;

impl ManifestReader for PyProjectManifest {
    fn kind(&self) -> ManifestKind { ManifestKind::PyProject }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let mut manifest_files: Vec<(PathBuf, &str)> = Vec::new();
        collect_python_manifests(project_root, &mut manifest_files, 0);
        if manifest_files.is_empty() { return None }

        let mut data = ManifestData::default();
        for (path, kind) in &manifest_files {
            let content = match std::fs::read_to_string(path) { Ok(c) => c, Err(_) => continue };
            let names = match *kind {
                "pyproject" => parse_pyproject_deps(&content),
                "requirements" => parse_requirements_txt(&content),
                "pipfile" => parse_pipfile_deps(&content),
                _ => Vec::new(),
            };
            for name in names {
                data.dependencies.insert(name);
            }
        }
        Some(data)
    }
}

fn collect_python_manifests<'a>(
    dir: &Path,
    out: &mut Vec<(PathBuf, &'a str)>,
    depth: usize,
) {
    if depth > 6 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if matches!(
                name.as_ref(),
                ".git" | "node_modules" | "target" | "__pycache__"
                    | ".venv" | "venv" | ".tox" | "dist" | "build" | ".eggs"
            ) { continue }
            collect_python_manifests(&path, out, depth + 1);
        } else {
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();
            let kind: &'a str = if name == "pyproject.toml" {
                "pyproject"
            } else if name == "requirements.txt"
                || (name.starts_with("requirements") && name.ends_with(".txt"))
            {
                "requirements"
            } else if name == "Pipfile" {
                "pipfile"
            } else { continue };
            out.push((path, kind));
        }
    }
}

/// Parse package names from `pyproject.toml` (PEP 621 + Poetry formats).
pub fn parse_pyproject_deps(content: &str) -> Vec<String> {
    let mut packages = Vec::new();
    let mut in_deps = false;
    let mut in_array = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_deps = matches!(
                trimmed,
                "[project.dependencies]"
                    | "[tool.poetry.dependencies]"
                    | "[tool.poetry.dev-dependencies]"
                    | "[tool.poetry.group.dev.dependencies]"
            ) || trimmed == "[project]";
            in_array = false;
            continue;
        }
        if trimmed.starts_with("dependencies") && trimmed.contains('=') {
            let rest = trimmed.splitn(2, '=').nth(1).unwrap_or("").trim();
            in_array = rest.starts_with('[') && !rest.contains(']');
            let data = if rest.starts_with('[') {
                let inner = rest.trim_start_matches('[');
                inner.trim_end_matches(']')
            } else { rest };
            for name in extract_pep508_names(data) { packages.push(name) }
            if rest.contains(']') { in_array = false }
            continue;
        }
        if in_array {
            if trimmed.starts_with(']') { in_array = false }
            for name in extract_pep508_names(trimmed) { packages.push(name) }
            continue;
        }
        if in_deps && !trimmed.starts_with('[') && trimmed.contains('=') {
            let key = trimmed.split('=').next().unwrap_or("").trim();
            if !key.is_empty()
                && key != "python"
                && key.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
            {
                packages.push(key.to_string());
            }
        }
    }
    packages
}

fn extract_pep508_names(s: &str) -> Vec<String> {
    let mut names = Vec::new();
    for part in s.split(',') {
        let part = part.trim().trim_matches(|c| c == '"' || c == '\'' || c == ']');
        let end = part
            .find(|c: char| matches!(c, '[' | '>' | '<' | '=' | '~' | '!' | ';' | '@' | ' '))
            .unwrap_or(part.len());
        let name = part[..end].trim();
        if !name.is_empty()
            && name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
        {
            names.push(name.to_string());
        }
    }
    names
}

pub fn parse_requirements_txt(content: &str) -> Vec<String> {
    let mut packages = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with('#')
            || trimmed.starts_with('-')
            || trimmed.starts_with("git+")
            || trimmed.starts_with("http")
        { continue }
        let without_comment = trimmed.split('#').next().unwrap_or(trimmed).trim();
        let end = without_comment
            .find(|c: char| matches!(c, '[' | '>' | '<' | '=' | '!' | ';' | '@' | ' '))
            .unwrap_or(without_comment.len());
        let name = without_comment[..end].trim();
        if !name.is_empty() { packages.push(name.to_string()) }
    }
    packages
}

pub fn parse_pipfile_deps(content: &str) -> Vec<String> {
    let mut packages = Vec::new();
    let mut in_section = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_section = matches!(trimmed, "[packages]" | "[dev-packages]");
            continue;
        }
        if !in_section || trimmed.is_empty() || trimmed.starts_with('#') { continue }
        if let Some(eq_pos) = trimmed.find('=') {
            let key = trimmed[..eq_pos].trim();
            if !key.is_empty()
                && key.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
            {
                packages.push(key.to_string());
            }
        }
    }
    packages
}

// ===========================================================================
// Discovery — site-packages probing
// ===========================================================================

pub fn discover_python_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    let manifest = PyProjectManifest;
    let Some(data) = manifest.read(project_root) else { return Vec::new() };
    if data.dependencies.is_empty() { return Vec::new() }

    let site_packages = find_python_site_packages(project_root);
    if site_packages.is_empty() {
        debug!("No Python site-packages discovered; skipping Python externals");
        return Vec::new();
    }
    debug!(
        "Probing {} site-packages root(s) for {} declared deps",
        site_packages.len(),
        data.dependencies.len()
    );

    let mut roots = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for dep_raw in &data.dependencies {
        let normalized = normalize_python_dep_name(dep_raw);
        if normalized.is_empty() { continue }
        let mut matched = false;
        for sp in &site_packages {
            let pkg_dir = sp.join(&normalized);
            if pkg_dir.is_dir() && !seen.contains(&pkg_dir) {
                seen.insert(pkg_dir.clone());
                roots.push(ExternalDepRoot {
                    module_path: normalized.clone(),
                    version: String::from("unknown"),
                    root: pkg_dir,
                    ecosystem: LEGACY_ECOSYSTEM_TAG,
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
                    ecosystem: LEGACY_ECOSYSTEM_TAG,
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

fn python_top_level_lookup(
    site_packages: &Path,
    normalized: &str,
    seen: &mut std::collections::HashSet<PathBuf>,
) -> Option<Vec<ExternalDepRoot>> {
    let entries = std::fs::read_dir(site_packages).ok()?;
    let lower_prefix = normalized.to_lowercase();
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_dir() { continue }
        let name_os = entry.file_name();
        let name = name_os.to_string_lossy();
        if !name.ends_with(".dist-info") { continue }
        let stem = name.trim_end_matches(".dist-info");
        let dist_part = stem.rsplit_once('-').map(|(d, _)| d).unwrap_or(stem);
        let dist_lower = dist_part.to_lowercase();
        if dist_lower != lower_prefix { continue }

        let top_level_path = entry.path().join("top_level.txt");
        let Ok(contents) = std::fs::read_to_string(&top_level_path) else {
            return Some(Vec::new());
        };
        let mut out = Vec::new();
        for line in contents.lines() {
            let import_name = line.trim();
            if import_name.is_empty() || import_name.starts_with('_') { continue }
            let pkg_dir = site_packages.join(import_name);
            if pkg_dir.is_dir() && !seen.contains(&pkg_dir) {
                seen.insert(pkg_dir.clone());
                out.push(ExternalDepRoot {
                    module_path: import_name.to_string(),
                    version: String::from("unknown"),
                    root: pkg_dir,
                    ecosystem: LEGACY_ECOSYSTEM_TAG,
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
                    ecosystem: LEGACY_ECOSYSTEM_TAG,
                    package_id: None,
                });
            }
        }
        return Some(out);
    }
    None
}

pub fn discover_python_externals_scoped(
    workspace_root: &Path,
    package_abs_path: &Path,
) -> Vec<ExternalDepRoot> {
    let manifest = PyProjectManifest;
    let Some(data) = manifest.read(package_abs_path) else { return Vec::new() };
    if data.dependencies.is_empty() { return Vec::new() }

    let site_packages = find_python_site_packages_with_ancestors(package_abs_path, workspace_root);
    if site_packages.is_empty() {
        debug!("No site-packages discovered for {}", package_abs_path.display());
        return Vec::new();
    }
    let mut roots = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for dep_raw in &data.dependencies {
        let normalized = normalize_python_dep_name(dep_raw);
        if normalized.is_empty() { continue }
        let mut matched = false;
        for sp in &site_packages {
            let pkg_dir = sp.join(&normalized);
            if pkg_dir.is_dir() && !seen.contains(&pkg_dir) {
                seen.insert(pkg_dir.clone());
                roots.push(ExternalDepRoot {
                    module_path: normalized.clone(),
                    version: String::from("unknown"),
                    root: pkg_dir,
                    ecosystem: LEGACY_ECOSYSTEM_TAG,
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
                    ecosystem: LEGACY_ECOSYSTEM_TAG,
                    package_id: None,
                });
                matched = true;
                break;
            }
        }
        if !matched {
            for sp in &site_packages {
                if let Some(r) = python_top_level_lookup(sp, &normalized, &mut seen) {
                    roots.extend(r);
                    break;
                }
            }
        }
    }
    roots
}

fn find_python_site_packages_with_ancestors(start: &Path, workspace_root: &Path) -> Vec<PathBuf> {
    if let Some(raw) = std::env::var_os("BEARWISDOM_PYTHON_SITE_PACKAGES") {
        let mut out = Vec::new();
        for seg in std::env::split_paths(&raw) {
            if seg.as_os_str().is_empty() { continue }
            if seg.is_dir() && !out.contains(&seg) { out.push(seg) }
        }
        if !out.is_empty() { return out }
    }
    let mut out: Vec<PathBuf> = Vec::new();
    let mut push_if_dir = |p: PathBuf, out: &mut Vec<PathBuf>| {
        if p.is_dir() && !out.contains(&p) { out.push(p) }
    };
    let mut current: Option<&Path> = Some(start);
    while let Some(dir) = current {
        for venv_name in &[".venv", "venv", ".env"] {
            let venv = dir.join(venv_name);
            if !venv.is_dir() { continue }
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
        if dir == workspace_root { break }
        current = dir.parent();
    }
    out
}

pub fn find_python_site_packages(project_root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut push_if_dir = |p: PathBuf, out: &mut Vec<PathBuf>| {
        if p.is_dir() && !out.contains(&p) { out.push(p) }
    };

    if let Some(raw) = std::env::var_os("BEARWISDOM_PYTHON_SITE_PACKAGES") {
        for seg in std::env::split_paths(&raw) {
            if seg.as_os_str().is_empty() { continue }
            push_if_dir(seg, &mut out);
        }
        if !out.is_empty() { return out }
    }

    let mut candidate_dirs: Vec<PathBuf> = vec![project_root.to_path_buf()];
    if let Ok(entries) = std::fs::read_dir(project_root) {
        for entry in entries.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) { continue }
            let name = entry.file_name();
            let name_lossy = name.to_string_lossy();
            if name_lossy.starts_with('.')
                || matches!(
                    name_lossy.as_ref(),
                    "node_modules" | "target" | "dist" | "build" | "__pycache__"
                )
            { continue }
            candidate_dirs.push(entry.path());
        }
    }

    for dir in &candidate_dirs {
        for venv_name in &[".venv", "venv", ".env"] {
            let venv = dir.join(venv_name);
            if !venv.is_dir() { continue }
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
    }

    if !out.is_empty() { return out }

    if let Some(home) = std::env::var_os("PYTHONHOME") {
        let base = PathBuf::from(home);
        push_if_dir(base.join("Lib").join("site-packages"), &mut out);
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
pub fn normalize_python_dep_name(raw: &str) -> String {
    let end = raw
        .find(|c: char| {
            matches!(c, '[' | '<' | '>' | '=' | '!' | '~' | ';' | ' ' | '\t' | '@')
        })
        .unwrap_or(raw.len());
    let name = &raw[..end];
    name.trim()
        .to_lowercase()
        .replace('-', "_")
        .replace('.', "_")
}

// ---------------------------------------------------------------------------
// Reachability: package entry + bounded relative-import expansion
// ---------------------------------------------------------------------------

const PY_REEXPORT_MAX_DEPTH: u32 = 3;

/// Start from the package's entry (`__init__.py` for directory packages, the
/// .py file itself for single-file modules) and follow relative imports
/// bounded at depth 3. Most packages surface their public API through
/// `__init__.py` with `from .sub import X` re-exports; this walks that
/// surface without indexing unrelated internal modules.
fn resolve_python_package_entry(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
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

fn expand_python_reexports_into(
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

// ---------------------------------------------------------------------------
// Walk
// ---------------------------------------------------------------------------

pub fn walk_python_external_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    if dep.root.is_file() {
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
        walk_python_dir_bounded(&dep.root, &dep.root, dep, &mut out, 0);
    }
    out
}

fn walk_python_dir_bounded(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= MAX_WALK_DEPTH { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else { continue };
        let path = entry.path();
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "__pycache__" | "tests" | "test" | ".git" | "_test") {
                    continue;
                }
                if name.ends_with(".dist-info") || name.ends_with(".egg-info") { continue }
            }
            walk_python_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".py") { continue }
            if name.starts_with("test_") || name.ends_with("_test.py") || name == "conftest.py" {
                continue;
            }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            out.push(WalkedFile {
                relative_path: format!("ext:py:{}/{}", dep.module_path, rel_sub),
                absolute_path: path,
                language: "python",
            });
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
    fn ecosystem_identity() {
        let p = PypiEcosystem;
        assert_eq!(p.id(), ID);
        assert_eq!(Ecosystem::kind(&p), EcosystemKind::Package);
        assert_eq!(Ecosystem::languages(&p), &["python"]);
    }

    #[test]
    fn legacy_locator_tag_is_python() {
        assert_eq!(ExternalSourceLocator::ecosystem(&PypiEcosystem), "python");
    }

    #[test]
    fn python_name_normalization_strips_extras_and_versions() {
        assert_eq!(normalize_python_dep_name("fastapi"), "fastapi");
        assert_eq!(normalize_python_dep_name("fastapi[standard]<1.0.0,>=0.114.2"), "fastapi");
        assert_eq!(normalize_python_dep_name("pydantic-settings>=2.2.1"), "pydantic_settings");
        assert_eq!(normalize_python_dep_name("SQLAlchemy>=2.0"), "sqlalchemy");
        assert_eq!(normalize_python_dep_name("psycopg[binary]<4.0.0,>=3.1.13"), "psycopg");
    }

    #[test]
    fn python_name_normalization_handles_environment_markers() {
        assert_eq!(normalize_python_dep_name("urllib3<2;python_version<'3.10'"), "urllib3");
        assert_eq!(normalize_python_dep_name("some-pkg @ git+https://github.com/x/y"), "some_pkg");
    }

    #[test]
    fn pyproject_pep621_array() {
        let toml = r#"
[project]
name = "test"
dependencies = [
    "fastapi>=0.100",
    "pydantic>=2",
    "sqlalchemy",
]
"#;
        let deps = parse_pyproject_deps(toml);
        assert!(deps.contains(&"fastapi".to_string()));
        assert!(deps.contains(&"pydantic".to_string()));
        assert!(deps.contains(&"sqlalchemy".to_string()));
    }

    #[test]
    fn pyproject_poetry_format() {
        let toml = r#"
[tool.poetry.dependencies]
python = "^3.10"
django = "^4.2"
celery = { extras = ["redis"], version = "^5.3" }
"#;
        let deps = parse_pyproject_deps(toml);
        assert!(deps.contains(&"django".to_string()));
        assert!(deps.contains(&"celery".to_string()));
        assert!(!deps.contains(&"python".to_string()));
    }

    #[test]
    fn requirements_txt_skips_comments_and_urls() {
        let content = "# comment\nrequests==2.28.0\n-r other.txt\nhttp://example.com/pkg.tar.gz\ngit+https://github.com/x/y.git\npandas>=1.5\n";
        let deps = parse_requirements_txt(content);
        assert_eq!(deps, vec!["requests", "pandas"]);
    }

    #[test]
    fn pipfile_parses_packages_section() {
        let content = r#"
[packages]
requests = "*"
pandas = {version = ">=1.5"}

[dev-packages]
pytest = "*"
"#;
        let deps = parse_pipfile_deps(content);
        assert!(deps.contains(&"requests".to_string()));
        assert!(deps.contains(&"pandas".to_string()));
        assert!(deps.contains(&"pytest".to_string()));
    }

    #[allow(dead_code)]
    fn _ensure_shared_locator_typed() -> Arc<dyn ExternalSourceLocator> {
        shared_locator()
    }
}
