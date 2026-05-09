// =============================================================================
// ecosystem/alire.rs — Alire ecosystem (Ada package manager)
//
// Alire is the modern Ada package manager. Each project declares deps in
// `alire.toml` via `[[depends-on]]` array-of-tables or a flat `[depends-on]`
// table. The CLI (`alr`) populates a per-user cache:
//
//   * Linux/macOS: `~/.cache/alire/releases/<crate>_<version>_<hash>/`
//   * Windows:     `%LOCALAPPDATA%\alire\cache\releases\<crate>_<version>_<hash>\`
//
// Each release directory has its own nested `alire.toml` and a `src/` (and
// sometimes `tests/`, examples) of Ada specs (`.ads`) and bodies (`.adb`).
// Activation: `ManifestMatch` on `alire.toml`. We do not probe-and-pray; if
// no manifest, no deps, no roots — same shape as cabal / cargo / npm.
// =============================================================================

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rayon::prelude::*;
use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
    SymbolLocationIndex,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("alire");

const MANIFESTS: &[ManifestSpec] = &[];
const LANGUAGES: &[&str] = &["ada"];
const LEGACY_ECOSYSTEM_TAG: &str = "alire";

pub struct AlireEcosystem;

// ---------------------------------------------------------------------------
// Ecosystem trait impl
// ---------------------------------------------------------------------------

impl Ecosystem for AlireEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Package }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }
    fn manifest_specs(&self) -> &'static [ManifestSpec] { MANIFESTS }

    fn workspace_package_files(&self) -> &'static [(&'static str, &'static str)] {
        &[("alire.toml", "ada")]
    }

    fn pruned_dir_names(&self) -> &'static [&'static str] {
        // `alire/` is the per-project build/cache directory Alire creates;
        // its contents are derived and shouldn't be walked as project source.
        &["alire", "obj", "lib"]
    }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::ManifestMatch
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_alire_externals(ctx.project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_alire_root(dep)
    }

    fn supports_reachability(&self) -> bool { true }
    // Eager walk: like gnat-stdlib, the Ada bare-name + use-clause shape
    // requires every public subprogram from a use'd package to live in
    // the symbol table. members_of() only sees indexed symbols, so
    // demand-driven loading would need engine wildcard-demand support
    // we don't have for Ada. The Alire dep cache is bounded (~1700 Ada
    // files in a typical workspace setup) so the eager cost is fine.

    fn build_symbol_index(&self, dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
        build_alire_symbol_index(dep_roots)
    }

    fn resolve_import(
        &self,
        dep: &ExternalDepRoot,
        package: &str,
        _symbols: &[&str],
    ) -> Vec<WalkedFile> {
        resolve_package(dep, package).into_iter().collect()
    }

    fn resolve_symbol(&self, dep: &ExternalDepRoot, fqn: &str) -> Vec<WalkedFile> {
        // `Ada.Calendar.Time_Zones.Local_Time_Offset` → walk back through
        // dotted segments and stop on the first one that names an indexed
        // package spec.
        let mut probe = fqn.to_string();
        while !probe.is_empty() {
            if let Some(walked) = resolve_package(dep, &probe) {
                return vec![walked];
            }
            match probe.rfind('.') {
                Some(idx) => probe.truncate(idx),
                None => break,
            }
        }
        Vec::new()
    }
}

// ---------------------------------------------------------------------------
// Legacy ExternalSourceLocator impl
// ---------------------------------------------------------------------------

impl ExternalSourceLocator for AlireEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_alire_externals(project_root)
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_alire_root(dep)
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<AlireEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(AlireEcosystem)).clone()
}

// ---------------------------------------------------------------------------
// Manifest reader
// ---------------------------------------------------------------------------

pub struct AlireManifest;

impl crate::ecosystem::manifest::ManifestReader for AlireManifest {
    fn kind(&self) -> crate::ecosystem::manifest::ManifestKind {
        crate::ecosystem::manifest::ManifestKind::Alire
    }

    fn read(&self, project_root: &Path) -> Option<crate::ecosystem::manifest::ManifestData> {
        let deps = parse_alire_dependencies(project_root);
        if deps.is_empty() { return None }
        let mut data = crate::ecosystem::manifest::ManifestData::default();
        data.dependencies = deps.into_iter().collect();
        Some(data)
    }
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

pub fn discover_alire_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    // Walk every `alire.toml` in the project tree, not just the root.
    // Multi-package Alire workspaces (root + nested `tests/`, `examples/`,
    // `support/`, …) declare deps independently — Septum's tests/alire.toml
    // pulls in `trendy_test` while the main alire.toml doesn't, so a
    // root-only scan misses the entire test framework.
    let declared = collect_alire_dependencies_recursive(project_root);
    if declared.is_empty() { return Vec::new() }

    let cache_roots = alire_cache_roots();
    let mut roots = Vec::new();

    for cache in &cache_roots {
        if !cache.is_dir() { continue }
        let Ok(entries) = std::fs::read_dir(cache) else { continue };
        // Group cache entries by crate name → highest version wins.
        let mut by_crate: std::collections::HashMap<String, Vec<(String, PathBuf)>> =
            std::collections::HashMap::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() { continue }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            // Cache layout: `<crate>_<version>_<hash>`. The hash is hex —
            // the *last* underscore separates hash from version.
            let Some((stem, _hash)) = name.rsplit_once('_') else { continue };
            // Strip trailing version: `<crate>_<version>` → `(<crate>, <version>)`.
            let (crate_name, version) = match stem.rsplit_once('_') {
                Some((c, v)) => (c, v),
                None => (stem, ""),
            };
            by_crate
                .entry(crate_name.to_string())
                .or_default()
                .push((version.to_string(), path));
        }

        for dep in &declared {
            let key = dep.replace('-', "_");
            let candidates = by_crate.get(&key).or_else(|| by_crate.get(dep));
            let Some(candidates) = candidates else { continue };
            // Pick the highest-version directory (lex sort works for the
            // typical SemVer-shaped values Alire publishes — `0.3.0`,
            // `1.1.0`, `26.0.0`, …).
            let mut sorted = candidates.clone();
            sorted.sort_by(|a, b| a.0.cmp(&b.0));
            if let Some((version, path)) = sorted.into_iter().next_back() {
                roots.push(ExternalDepRoot {
                    module_path: dep.clone(),
                    version,
                    root: path,
                    ecosystem: LEGACY_ECOSYSTEM_TAG,
                    package_id: None,
                    requested_imports: Vec::new(),
                });
            }
        }
    }

    debug!(
        "alire: {} roots resolved ({} declared deps, {} cache locations)",
        roots.len(),
        declared.len(),
        cache_roots.len()
    );
    roots
}

/// Cache roots Alire `alr` writes deployed releases under. The
/// `BEARWISDOM_ALIRE_CACHE` env override is exclusive — when set, default
/// platform paths are skipped so test fixtures don't pick up the real
/// per-user cache, and CI runs stay deterministic.
fn alire_cache_roots() -> Vec<PathBuf> {
    if let Some(explicit) = std::env::var_os("BEARWISDOM_ALIRE_CACHE") {
        let p = PathBuf::from(explicit);
        if p.is_dir() {
            // Caller may point at the cache root or the releases sub-dir.
            let candidate = if p.file_name().and_then(|n| n.to_str()) == Some("releases") {
                p
            } else {
                p.join("releases")
            };
            return vec![candidate];
        }
        return Vec::new();
    }

    let mut bases = Vec::new();

    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        bases.push(
            PathBuf::from(local)
                .join("alire")
                .join("cache")
                .join("releases"),
        );
    }

    if let Some(home) = dirs::home_dir() {
        bases.push(home.join(".cache").join("alire").join("releases"));
        bases.push(
            home.join("Library")
                .join("Caches")
                .join("alire")
                .join("releases"),
        );
    }

    if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME") {
        bases.push(PathBuf::from(xdg).join("alire").join("releases"));
    }

    bases
}

// ---------------------------------------------------------------------------
// Manifest parsing — line-based TOML scan
// ---------------------------------------------------------------------------

/// Parse `alire.toml` at the project root and return its `[[depends-on]]` /
/// `[depends-on]` declarations. Used by the manifest reader where we want
/// only the workspace-root manifest's deps. For ecosystem discovery use
/// `collect_alire_dependencies_recursive` instead — it walks nested
/// alire.toml files (tests/, examples/, support/, …) so deps declared in
/// sub-package manifests are picked up too.
pub fn parse_alire_dependencies(project_root: &Path) -> Vec<String> {
    let manifest = project_root.join("alire.toml");
    let Ok(content) = std::fs::read_to_string(&manifest) else { return Vec::new() };
    parse_alire_dependencies_text(&content)
}

/// Walk the project tree (bounded depth + standard prunes) collecting
/// dependency names from every `alire.toml` found. Multi-package
/// workspaces declare deps in nested manifests (the canonical Septum
/// shape), so a root-only scan loses test-framework / example-only deps.
pub fn collect_alire_dependencies_recursive(project_root: &Path) -> Vec<String> {
    let mut deps: HashSet<String> = HashSet::new();
    walk_alire_manifests(project_root, &mut deps, 0);
    let mut v: Vec<String> = deps.into_iter().collect();
    v.sort();
    v
}

fn walk_alire_manifests(dir: &Path, deps: &mut HashSet<String>, depth: u32) {
    if depth >= MAX_WALK_DEPTH { return }
    let manifest = dir.join("alire.toml");
    if manifest.is_file() {
        if let Ok(content) = std::fs::read_to_string(&manifest) {
            for d in parse_alire_dependencies_text(&content) {
                deps.insert(d);
            }
        }
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_dir() { continue }
        let path = entry.path();
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if matches!(name, "obj" | "lib" | "alire" | ".git" | "node_modules" | "target" | ".bearwisdom")
                || name.starts_with('.')
            {
                continue;
            }
        }
        walk_alire_manifests(&path, deps, depth + 1);
    }
}

/// Same as `parse_alire_dependencies`, but operating on already-loaded
/// manifest text. Split out for unit testing.
pub fn parse_alire_dependencies_text(content: &str) -> Vec<String> {
    let mut deps: HashSet<String> = HashSet::new();
    let mut in_depends = false;

    for raw in content.lines() {
        let line = strip_toml_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            // Section header: track whether we just entered a depends-on
            // table (single or array-of-tables).
            in_depends = is_depends_on_header(line);
            continue;
        }
        if !in_depends {
            continue;
        }
        // Inside [[depends-on]] / [depends-on], each `<name> = "..."` line
        // declares one dep.
        if let Some((key, _)) = line.split_once('=') {
            let name = key.trim().trim_matches(|c: char| c == '"' || c == '\'');
            if is_valid_dep_name(name) {
                deps.insert(name.to_string());
            }
        }
    }

    // Stable order for deterministic discovery output.
    let mut v: Vec<String> = deps.into_iter().collect();
    v.sort();
    v
}

fn is_depends_on_header(line: &str) -> bool {
    // Strip surrounding `[`/`]` (one or two layers for arrays-of-tables).
    let inner = line
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('[');
    inner.trim() == "depends-on"
}

fn is_valid_dep_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn strip_toml_comment(line: &str) -> &str {
    // Naive: TOML comment intro is `#` outside a string. alire.toml dep
    // lines never include `#` inside a quoted version string in practice,
    // so a literal scan is good enough.
    match line.find('#') {
        Some(idx) => &line[..idx],
        None => line,
    }
}

// ---------------------------------------------------------------------------
// Walker
// ---------------------------------------------------------------------------

fn walk_alire_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_dir_bounded(&dep.root, &dep.root, dep, &mut out, 0);
    out
}

fn walk_dir_bounded(
    dir: &Path,
    root: &Path,
    dep: &ExternalDepRoot,
    out: &mut Vec<WalkedFile>,
    depth: u32,
) {
    if depth >= MAX_WALK_DEPTH { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "obj" | "lib" | "alire" | "tests" | "test" | "examples")
                    || name.starts_with('.')
                {
                    continue;
                }
            }
            walk_dir_bounded(&path, root, dep, out, depth + 1);
        } else if ft.is_file() {
            let Some(ext) = path.extension().and_then(|e| e.to_str()) else { continue };
            if ext != "ads" && ext != "adb" { continue }
            let rel = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            let virtual_path = format!("ext:alire:{}/{}", dep.module_path, rel);
            out.push(WalkedFile {
                relative_path: virtual_path,
                absolute_path: path,
                language: "ada",
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Demand-driven resolution
// ---------------------------------------------------------------------------

fn resolve_package(dep: &ExternalDepRoot, package: &str) -> Option<WalkedFile> {
    let needle = package.to_ascii_lowercase();
    let mut hit: Option<PathBuf> = None;
    walk_for_package(&dep.root, &needle, &mut hit, 0);
    let path = hit?;
    Some(make_walked_file(dep, &path))
}

fn walk_for_package(
    dir: &Path,
    needle: &str,
    hit: &mut Option<PathBuf>,
    depth: u32,
) {
    if hit.is_some() { return }
    if depth >= MAX_WALK_DEPTH { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        if hit.is_some() { return }
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "obj" | "lib" | "alire" | "tests" | "test" | "examples")
                    || name.starts_with('.')
                {
                    continue;
                }
            }
            walk_for_package(&path, needle, hit, depth + 1);
        } else if ft.is_file() {
            let Some(ext) = path.extension().and_then(|e| e.to_str()) else { continue };
            if ext != "ads" { continue }
            if let Some(decl) = scan_package_decl(&path) {
                if decl.eq_ignore_ascii_case(needle) {
                    *hit = Some(path);
                    return;
                }
            }
        }
    }
}

fn make_walked_file(dep: &ExternalDepRoot, path: &Path) -> WalkedFile {
    let rel = path
        .strip_prefix(&dep.root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| {
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
    WalkedFile {
        relative_path: format!("ext:alire:{}/{}", dep.module_path, rel),
        absolute_path: path.to_path_buf(),
        language: "ada",
    }
}

// ---------------------------------------------------------------------------
// Symbol index — scan each `.ads` for its `package <Name>` declaration
// ---------------------------------------------------------------------------

pub(crate) fn build_alire_symbol_index(
    dep_roots: &[ExternalDepRoot],
) -> SymbolLocationIndex {
    let work: Vec<(String, PathBuf)> = dep_roots
        .iter()
        .flat_map(|dep| {
            let mut found = Vec::new();
            collect_ads_files(&dep.root, &mut found, 0);
            found
                .into_iter()
                .map(move |p| (dep.module_path.clone(), p))
        })
        .collect();

    if work.is_empty() {
        return SymbolLocationIndex::new();
    }

    let pairs: Vec<(String, String, PathBuf)> = work
        .par_iter()
        .filter_map(|(module, path)| {
            scan_package_decl(path).map(|qname| (module.clone(), qname, path.clone()))
        })
        .collect();

    let mut index = SymbolLocationIndex::new();
    for (module, qname, file) in pairs {
        let key = qname.to_ascii_lowercase();
        index.insert(module.clone(), key.clone(), file.clone());
        if key != qname {
            index.insert(module, qname, file);
        }
    }
    index
}

fn collect_ads_files(dir: &Path, out: &mut Vec<PathBuf>, depth: u32) {
    if depth >= MAX_WALK_DEPTH { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "obj" | "lib" | "alire" | "tests" | "test" | "examples")
                    || name.starts_with('.')
                {
                    continue;
                }
            }
            collect_ads_files(&path, out, depth + 1);
        } else if ft.is_file() {
            if path.extension().and_then(|e| e.to_str()) == Some("ads") {
                out.push(path);
            }
        }
    }
}

/// Read an `.ads` file and return the qualified name of the package it
/// declares. Recognises `package`, `private package`, `generic package`,
/// and `package body` forms. Same shape as `gnat_stdlib::scan_package_decl`
/// — duplicated here rather than cross-imported because the two ecosystems
/// are independent and may diverge (e.g. Alire's protected packages).
fn scan_package_decl(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    for raw in content.lines() {
        let line = strip_ada_comment(raw).trim();
        if line.is_empty() { continue }
        let mut tail = line;
        for prefix in ["private ", "generic "] {
            if let Some(rest) = tail.strip_prefix(prefix) {
                tail = rest.trim_start();
            }
        }
        let after_kw = if let Some(r) = tail.strip_prefix("package body ") {
            r
        } else if let Some(r) = tail.strip_prefix("package ") {
            r
        } else {
            continue;
        };
        let qname: String = after_kw
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '.' || *c == '_')
            .collect();
        if qname.is_empty() { continue }
        return Some(qname);
    }
    None
}

fn strip_ada_comment(line: &str) -> &str {
    match line.find("--") {
        Some(idx) => &line[..idx],
        None => line,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "alire_tests.rs"]
mod tests;
