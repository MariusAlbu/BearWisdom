// =============================================================================
// ecosystem/cargo.rs — Cargo ecosystem (Rust)
//
// Phase 2 + 3 combined: consolidates the external-source locator
// (`indexer/externals/rust_lang.rs`) and the manifest reader
// (`indexer/manifest/cargo.rs`) into a single ecosystem module. Rust is a
// single-language ecosystem; the multi-language consolidation pattern used
// by Maven/npm/Hex still applies here — just with one entry in
// `languages()`.
//
// Before: externals/rust_lang.rs + manifest/cargo.rs (892 LOC total).
// After:  ecosystem/cargo.rs (~700 LOC) — deduplicated; `CargoManifest`
// still implements `ManifestReader` so the existing manifest registry
// (`indexer/manifest/mod.rs::all_readers()`) keeps working. Module path
// for the manifest reader and parser functions updates from
// `crate::ecosystem::manifest::cargo` → `crate::ecosystem::cargo`.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rayon::prelude::*;
use tracing::debug;
use tree_sitter::{Node, Parser};

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
    SymbolLocationIndex,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::ecosystem::manifest::{ManifestData, ManifestKind, ManifestReader, ReaderEntry};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("cargo");

const MANIFESTS: &[ManifestSpec] = &[];
const LANGUAGES: &[&str] = &["rust"];
const LEGACY_ECOSYSTEM_TAG: &str = "rust";

pub struct CargoEcosystem;

// ---------------------------------------------------------------------------
// Ecosystem trait impl
// ---------------------------------------------------------------------------

impl Ecosystem for CargoEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Package }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }
    fn manifest_specs(&self) -> &'static [ManifestSpec] { MANIFESTS }

    fn workspace_package_files(&self) -> &'static [(&'static str, &'static str)] {
        &[("Cargo.toml", "cargo")]
    }

    fn pruned_dir_names(&self) -> &'static [&'static str] {
        &["target"]
    }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Any(&[
            EcosystemActivation::ManifestMatch,
            EcosystemActivation::LanguagePresent("rust"),
        ])
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_cargo_roots(ctx.project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_cargo_root(dep)
    }

    fn supports_reachability(&self) -> bool { true }

    fn resolve_import(
        &self,
        dep: &ExternalDepRoot,
        _package: &str,
        _symbols: &[&str],
    ) -> Vec<WalkedFile> {
        // Start from the crate's library entry (`src/lib.rs`; fall back to
        // `src/main.rs` for binary-only crates that still get imported) and
        // follow `mod X;` declarations bounded at depth 3. Internal `mod`
        // declarations are included because `pub use internal::Foo` re-
        // exports can expose items that live in non-pub modules.
        resolve_crate_entry(dep)
    }

    fn resolve_symbol(
        &self,
        dep: &ExternalDepRoot,
        _fqn: &str,
    ) -> Vec<WalkedFile> {
        // Same entry as resolve_import — the crate surface is fully defined
        // by `src/lib.rs` plus its module tree; fqn-specific walking is a
        // later optimization.
        resolve_crate_entry(dep)
    }

    fn build_symbol_index(
        &self,
        dep_roots: &[ExternalDepRoot],
    ) -> SymbolLocationIndex {
        build_cargo_symbol_index(dep_roots)
    }

    fn uses_demand_driven_parse(&self) -> bool { true }
}

// ---------------------------------------------------------------------------
// Legacy ExternalSourceLocator impl
// ---------------------------------------------------------------------------

impl ExternalSourceLocator for CargoEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_cargo_roots(project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_cargo_root(dep)
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<CargoEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(CargoEcosystem)).clone()
}

// ===========================================================================
// Manifest reader — migrated from indexer/manifest/cargo.rs
// ===========================================================================

/// `CargoManifest` reads `Cargo.toml` + `Cargo.lock` per-package during
/// `ProjectContext` building. Still lives as a `ManifestReader` impl so the
/// existing `manifest::all_readers()` registry continues to dispatch it.
/// Phase 4 (ProjectContext wiring) collapses this path into an Ecosystem-
/// native manifest flow.
pub struct CargoManifest;

impl ManifestReader for CargoManifest {
    fn kind(&self) -> ManifestKind {
        ManifestKind::Cargo
    }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let entries = self.read_all(project_root);
        if entries.is_empty() { return None }
        let mut data = ManifestData::default();
        for e in &entries {
            data.dependencies.extend(e.data.dependencies.iter().cloned());
        }
        Some(data)
    }

    fn read_all(&self, project_root: &Path) -> Vec<ReaderEntry> {
        let mut paths = Vec::new();
        collect_cargo_tomls(project_root, &mut paths, 0);

        let mut out = Vec::new();
        for manifest_path in paths {
            let Ok(content) = std::fs::read_to_string(&manifest_path) else { continue };

            let mut data = ManifestData::default();
            for name in parse_cargo_dependencies(&content) {
                data.dependencies.insert(name);
            }
            for key in parse_cargo_path_dependencies(&content) {
                if !data.project_refs.contains(&key) {
                    data.project_refs.push(key);
                }
            }

            let name = parse_cargo_package_name(&content);
            let package_dir = manifest_path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| project_root.to_path_buf());

            out.push(ReaderEntry {
                package_dir,
                manifest_path,
                data,
                name,
            });
        }
        out
    }
}

fn collect_cargo_tomls(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 8 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if matches!(
                name.as_ref(),
                "target" | ".git" | "node_modules" | "bin" | "obj" | ".cargo"
            ) { continue }
            collect_cargo_tomls(&path, out, depth + 1);
        } else if entry.file_name() == "Cargo.toml" {
            out.push(path);
        }
    }
}

/// Parse crate names from `[dependencies]` + `[dev-dependencies]` +
/// `[build-dependencies]` + `[workspace.dependencies]` sections.
///
/// Line-by-line scan — avoids a full TOML dependency. Handles
/// `serde = "1"`, `tokio = { ... }`, `foo.workspace = true`.
pub fn parse_cargo_dependencies(content: &str) -> Vec<String> {
    let mut crates = Vec::new();
    let mut in_dep_section = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_dep_section = matches!(
                trimmed,
                "[dependencies]"
                    | "[dev-dependencies]"
                    | "[build-dependencies]"
                    | "[workspace.dependencies]"
            );
            continue;
        }
        if !in_dep_section { continue }
        if trimmed.is_empty() || trimmed.starts_with('#') { continue }

        if let Some(eq_pos) = trimmed.find('=') {
            let key = trimmed[..eq_pos]
                .trim()
                .split('.')
                .next()
                .unwrap_or("")
                .trim();
            if !key.is_empty()
                && key.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
            {
                crates.push(key.to_string());
            }
        }
    }
    crates
}

/// Parse sibling-workspace crate names from `path = "..."` dependency entries.
/// Each entry yields the dep KEY (the Cargo-side name), not the target crate's
/// `[package].name` — the key is what appears in `use foo::...` source code.
pub fn parse_cargo_path_dependencies(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_dep_section = false;
    let mut pending_key: Option<String> = None;
    let mut pending_table = String::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_dep_section = matches!(
                trimmed,
                "[dependencies]"
                    | "[dev-dependencies]"
                    | "[build-dependencies]"
                    | "[workspace.dependencies]"
            );
            pending_key = None;
            pending_table.clear();
            continue;
        }
        if !in_dep_section { continue }
        if trimmed.is_empty() || trimmed.starts_with('#') { continue }

        if let Some(key) = pending_key.clone() {
            pending_table.push(' ');
            pending_table.push_str(trimmed);
            if trimmed.contains('}') {
                if pending_table.contains("path") && pending_table.contains('=') {
                    if !out.contains(&key) { out.push(key) }
                }
                pending_key = None;
                pending_table.clear();
            }
            continue;
        }

        let Some(eq) = trimmed.find('=') else { continue };
        let key = trimmed[..eq]
            .trim()
            .split('.')
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        if key.is_empty()
            || !key.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        { continue }
        let value = trimmed[eq + 1..].trim();
        if value.starts_with('{') && value.ends_with('}') {
            if value.contains("path") && value.contains('=') {
                if !out.contains(&key) { out.push(key) }
            }
            continue;
        }
        if value.starts_with('{') {
            pending_key = Some(key);
            pending_table.push_str(value);
            continue;
        }
    }
    out
}

fn parse_cargo_package_name(content: &str) -> Option<String> {
    let mut in_package = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_package = trimmed == "[package]";
            continue;
        }
        if !in_package { continue }
        if let Some(rest) = trimmed.strip_prefix("name") {
            let rest = rest.trim_start();
            let Some(rest) = rest.strip_prefix('=') else { continue };
            let rest = rest.trim();
            let Some(rest) = rest.strip_prefix('"') else { continue };
            let Some(end) = rest.find('"') else { continue };
            return Some(rest[..end].to_string());
        }
    }
    None
}

// ===========================================================================
// Discovery — Cargo.lock → ~/.cargo/registry/src/<index>/<name>-<ver>/
// ===========================================================================

#[derive(Debug, Clone)]
struct CargoLockEntry {
    name: String,
    version: String,
}

/// Parse `[[package]]` entries from `Cargo.lock`. Only returns packages with
/// `source = "registry+..."` — workspace members and git deps are omitted.
fn parse_cargo_lock(content: &str) -> Vec<CargoLockEntry> {
    let mut entries = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current_version: Option<String> = None;
    let mut current_is_registry = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "[[package]]" {
            if current_is_registry {
                if let (Some(name), Some(version)) = (current_name.take(), current_version.take()) {
                    entries.push(CargoLockEntry { name, version });
                }
            } else {
                current_name = None;
                current_version = None;
            }
            current_is_registry = false;
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with('#') { continue }
        let Some(eq) = trimmed.find(" = ") else { continue };
        let key = trimmed[..eq].trim();
        let rest = trimmed[eq + 3..].trim();
        let value = rest.trim_matches('"');
        match key {
            "name" => { current_name = Some(value.to_string()); }
            "version" => { current_version = Some(value.to_string()); }
            "source" => { current_is_registry = value.starts_with("registry+"); }
            _ => {}
        }
    }
    if current_is_registry {
        if let (Some(name), Some(version)) = (current_name, current_version) {
            entries.push(CargoLockEntry { name, version });
        }
    }
    entries
}

fn find_cargo_lock(start: &Path) -> Option<PathBuf> {
    let mut current = start;
    for _ in 0..8 {
        let lock = current.join("Cargo.lock");
        if lock.is_file() { return Some(lock) }
        current = current.parent()?;
    }
    None
}

fn find_cargo_lock_descend(start: &Path) -> Option<PathBuf> {
    find_cargo_lock_descend_bounded(start, 0)
}

fn find_cargo_lock_descend_bounded(dir: &Path, depth: u8) -> Option<PathBuf> {
    if depth > 2 { return None }
    let Ok(entries) = std::fs::read_dir(dir) else { return None };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.file_name().and_then(|n| n.to_str()) == Some("Cargo.lock") {
            return Some(path);
        }
        if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "target" | ".git" | "node_modules") || name.starts_with('.') {
                    continue;
                }
            }
            if let Some(found) = find_cargo_lock_descend_bounded(&path, depth + 1) {
                return Some(found);
            }
        }
    }
    None
}

fn cargo_registry_src_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let src_root = if let Ok(home) = std::env::var("CARGO_HOME") {
        PathBuf::from(home).join("registry").join("src")
    } else if let Some(home) = dirs::home_dir() {
        home.join(".cargo").join("registry").join("src")
    } else {
        return dirs;
    };
    if !src_root.is_dir() { return dirs }
    if let Ok(entries) = std::fs::read_dir(&src_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() { dirs.push(path) }
        }
    }
    dirs
}

fn split_crate_dir_name(s: &str) -> Option<(String, String)> {
    let bytes = s.as_bytes();
    let mut i = s.len();
    while let Some(pos) = s[..i].rfind('-') {
        if bytes.get(pos + 1).map_or(false, |b| b.is_ascii_digit()) {
            return Some((s[..pos].to_string(), s[pos + 1..].to_string()));
        }
        i = pos;
    }
    None
}

fn discover_cargo_roots(project_root: &Path) -> Vec<ExternalDepRoot> {
    let lock_path = find_cargo_lock(project_root)
        .or_else(|| find_cargo_lock_descend(project_root));

    let packages: Vec<CargoLockEntry> = if let Some(ref lp) = lock_path {
        if let Ok(content) = std::fs::read_to_string(lp) {
            let parsed = parse_cargo_lock(&content);
            if !parsed.is_empty() {
                debug!("Rust: loaded {} packages from {}", parsed.len(), lp.display());
                parsed
            } else { Vec::new() }
        } else { Vec::new() }
    } else { Vec::new() };

    let use_fallback = packages.is_empty();
    let toml_names: Vec<String> = if use_fallback {
        let cargo_toml = project_root.join("Cargo.toml");
        if !cargo_toml.is_file() { return Vec::new() }
        match std::fs::read_to_string(&cargo_toml) {
            Ok(content) => {
                let deps = parse_cargo_dependencies(&content);
                if deps.is_empty() { return Vec::new() }
                debug!("Rust: no Cargo.lock; {} declared deps from Cargo.toml", deps.len());
                deps
            }
            Err(_) => return Vec::new(),
        }
    } else { Vec::new() };

    let src_dirs = cargo_registry_src_dirs();
    if src_dirs.is_empty() {
        debug!("Rust: no ~/.cargo/registry/src found; skipping");
        return Vec::new();
    }

    let mut all_crate_dirs: Vec<PathBuf> = Vec::new();
    for src_dir in &src_dirs {
        if let Ok(entries) = std::fs::read_dir(src_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() { all_crate_dirs.push(path) }
            }
        }
    }

    let mut roots = Vec::new();

    if use_fallback {
        for crate_name in &toml_names {
            let prefix = format!("{crate_name}-");
            let mut matches: Vec<PathBuf> = all_crate_dirs
                .iter()
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .map(|s| {
                            s.starts_with(&prefix)
                                && s[prefix.len()..]
                                    .chars()
                                    .next()
                                    .map_or(false, |c| c.is_ascii_digit())
                        })
                        .unwrap_or(false)
                })
                .cloned()
                .collect();
            matches.sort();
            if let Some(best) = matches.pop() {
                let version = best
                    .file_name()
                    .and_then(|n| n.to_str())
                    .and_then(|n| n.strip_prefix(&prefix))
                    .unwrap_or("")
                    .to_string();
                roots.push(ExternalDepRoot {
                    module_path: crate_name.clone(),
                    version,
                    root: best,
                    ecosystem: LEGACY_ECOSYSTEM_TAG,
                    package_id: None,
                    requested_imports: Vec::new(),
                });
            }
        }
    } else {
        let mut dir_index: std::collections::HashMap<(String, String), PathBuf> =
            std::collections::HashMap::with_capacity(all_crate_dirs.len());

        for path in &all_crate_dirs {
            let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if let Some((name, version)) = split_crate_dir_name(dir_name) {
                dir_index.entry((name, version)).or_insert_with(|| path.clone());
            }
        }

        for entry in &packages {
            let key = (entry.name.clone(), entry.version.clone());
            let under_key = (entry.name.replace('-', "_"), entry.version.clone());
            let found = dir_index
                .get(&key)
                .or_else(|| dir_index.get(&under_key))
                .cloned();
            if let Some(crate_root) = found {
                roots.push(ExternalDepRoot {
                    module_path: entry.name.clone(),
                    version: entry.version.clone(),
                    root: crate_root,
                    ecosystem: LEGACY_ECOSYSTEM_TAG,
                    package_id: None,
                    requested_imports: Vec::new(),
                });
            }
        }
    }

    debug!("Rust: discovered {} external crate roots", roots.len());
    roots
}

// ---------------------------------------------------------------------------
// Reachability: crate entry + bounded `mod X;` expansion
// ---------------------------------------------------------------------------

const RS_MOD_MAX_DEPTH: u32 = 3;

/// Start from `src/lib.rs` (falling back to `src/main.rs` for binary-only
/// crates) and recursively follow `mod X;` declarations into the matching
/// source files. Each file yields zero or more child modules; bounded at
/// depth 3 so deeply nested crates still get their top surface without
/// walking every internal module.
fn resolve_crate_entry(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let src = dep.root.join("src");
    let entry = if src.join("lib.rs").is_file() {
        src.join("lib.rs")
    } else if src.join("main.rs").is_file() {
        src.join("main.rs")
    } else {
        return Vec::new();
    };

    let mut out = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    expand_rust_mods_into(dep, &dep.root, &entry, &mut out, &mut seen, 0);
    out
}

fn expand_rust_mods_into(
    dep: &ExternalDepRoot,
    crate_root: &Path,
    file: &Path,
    out: &mut Vec<WalkedFile>,
    seen: &mut std::collections::HashSet<PathBuf>,
    depth: u32,
) {
    if !seen.insert(file.to_path_buf()) { return }
    if !file.is_file() { return }

    let rel_sub = match file.strip_prefix(crate_root) {
        Ok(p) => p.to_string_lossy().replace('\\', "/"),
        Err(_) => file
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("mod.rs")
            .to_string(),
    };
    out.push(WalkedFile {
        relative_path: format!("ext:rust:{}/{}", dep.module_path, rel_sub),
        absolute_path: file.to_path_buf(),
        language: "rust",
    });

    if depth >= RS_MOD_MAX_DEPTH { return }

    let Ok(src) = std::fs::read_to_string(file) else { return };
    for child in extract_rust_mod_decls(&src) {
        let Some(next) = resolve_rust_mod_path(file, &child) else { continue };
        expand_rust_mods_into(dep, crate_root, &next, out, seen, depth + 1);
    }
}

/// Scan line-oriented for `mod X;` and `pub mod X;` (including
/// `pub(crate) mod X;` and similar visibility modifiers). Inline `mod X {
/// ... }` bodies are skipped — their contents are already in the same file.
fn extract_rust_mod_decls(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in src.lines() {
        let line = raw.trim_start();
        // Strip line comments.
        let line = match line.find("//") {
            Some(ix) => &line[..ix],
            None => line,
        };
        let line = line.trim();
        if !line.ends_with(';') { continue }

        let mut rest = line;
        if let Some(r) = rest.strip_prefix("pub") {
            rest = r.trim_start();
            // Optional visibility qualifier: pub(crate), pub(super), pub(in path)
            if let Some(r) = rest.strip_prefix('(') {
                let Some(close) = r.find(')') else { continue };
                rest = r[close + 1..].trim_start();
            }
        }

        let Some(r) = rest.strip_prefix("mod") else { continue };
        // Ensure the next char is whitespace — avoid matching `model;` etc.
        let after = r;
        if !after.starts_with(|c: char| c.is_whitespace()) { continue }
        let ident = after.trim_start().trim_end_matches(';').trim();
        if ident.is_empty() { continue }
        if !ident.chars().all(|c| c.is_alphanumeric() || c == '_') { continue }
        out.push(ident.to_string());
    }
    out
}

/// Resolve `mod X;` declared in `from_file` to either `X.rs` or `X/mod.rs`
/// relative to the owning module directory (file's parent for lib.rs/
/// main.rs/mod.rs; sibling directory named after the stem otherwise).
fn resolve_rust_mod_path(from_file: &Path, child: &str) -> Option<PathBuf> {
    let parent = from_file.parent()?;
    let stem = from_file.file_stem().and_then(|s| s.to_str())?;
    let mod_dir = if stem == "lib" || stem == "main" || stem == "mod" {
        parent.to_path_buf()
    } else {
        parent.join(stem)
    };

    let as_file = mod_dir.join(format!("{child}.rs"));
    if as_file.is_file() { return Some(as_file) }
    let as_mod = mod_dir.join(child).join("mod.rs");
    if as_mod.is_file() { return Some(as_mod) }
    None
}

// ---------------------------------------------------------------------------
// Walk
// ---------------------------------------------------------------------------

fn walk_cargo_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    let src = dep.root.join("src");
    let walk_root = if src.is_dir() { src } else { dep.root.clone() };
    walk_dir_bounded(&walk_root, &dep.root, dep, &mut out, 0);
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
        let Ok(file_type) = entry.file_type() else { continue };
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "tests" | "test" | "benches" | "examples" | "target")
                    || name.starts_with('.')
                { continue }
            }
            walk_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".rs") { continue }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            out.push(WalkedFile {
                relative_path: format!("ext:rust:{}/{}", dep.module_path, rel_sub),
                absolute_path: path,
                language: "rust",
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Symbol-location index (demand-driven pipeline entry)
// ---------------------------------------------------------------------------

pub(crate) fn build_cargo_symbol_index(dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
    let mut work: Vec<(String, WalkedFile)> = Vec::new();
    for dep in dep_roots {
        for wf in walk_cargo_root(dep) {
            work.push((dep.module_path.clone(), wf));
        }
    }
    if work.is_empty() {
        return SymbolLocationIndex::new();
    }
    let per_file: Vec<Vec<(String, String, PathBuf)>> = work
        .par_iter()
        .map(|(module, wf)| {
            let Ok(src) = std::fs::read_to_string(&wf.absolute_path) else {
                return Vec::new();
            };
            scan_rust_header(&src)
                .into_iter()
                .map(|name| (module.clone(), name, wf.absolute_path.clone()))
                .collect()
        })
        .collect();
    let mut index = SymbolLocationIndex::new();
    for batch in per_file {
        for (module, name, file) in batch {
            index.insert(module, name, file);
        }
    }
    index
}

/// Header-only tree-sitter scan of a Rust source file. Returns top-level
/// item names — structs, enums, unions, traits, type aliases, functions,
/// constants, statics, macros. Function / method / impl bodies are never
/// descended; we record `ReceiverType::method_name` inside `impl` blocks so
/// the chain walker can locate methods the same way it does on Go.
fn scan_rust_header(source: &str) -> Vec<String> {
    let language = tree_sitter_rust::LANGUAGE.into();
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let root = tree.root_node();
    let bytes = source.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        collect_rust_top_level_name(&child, bytes, &mut out);
    }
    out
}

fn collect_rust_top_level_name(node: &Node, bytes: &[u8], out: &mut Vec<String>) {
    match node.kind() {
        "function_item"
        | "function_signature_item"
        | "struct_item"
        | "union_item"
        | "enum_item"
        | "trait_item"
        | "type_item"
        | "const_item"
        | "static_item"
        | "mod_item"
        | "macro_definition" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(bytes) {
                    out.push(name.to_string());
                }
            }
        }
        "impl_item" => {
            // `impl Foo { fn bar() {} }` — surface the receiver name plus each
            // associated item so methods are locatable as `Foo::bar`.
            let recv = node.child_by_field_name("type").and_then(|t| {
                rust_type_identifier(&t, bytes)
            });
            if let Some(body) = node.child_by_field_name("body") {
                let mut cursor = body.walk();
                for inner in body.children(&mut cursor) {
                    if matches!(
                        inner.kind(),
                        "function_item"
                            | "function_signature_item"
                            | "associated_type"
                            | "const_item"
                    ) {
                        if let Some(name_node) = inner.child_by_field_name("name") {
                            if let Ok(name) = name_node.utf8_text(bytes) {
                                out.push(name.to_string());
                                if let Some(recv) = recv.as_ref() {
                                    out.push(format!("{recv}::{name}"));
                                }
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

/// Extract a simple type identifier from `impl <Type>`'s type node, unwrapping
/// generic and reference wrappers. Returns None for unrecognized shapes.
fn rust_type_identifier(node: &Node, bytes: &[u8]) -> Option<String> {
    match node.kind() {
        "type_identifier" => node.utf8_text(bytes).ok().map(String::from),
        "generic_type" | "reference_type" | "scoped_type_identifier" => {
            let mut cursor = node.walk();
            for inner in node.children(&mut cursor) {
                if let Some(name) = rust_type_identifier(&inner, bytes) {
                    return Some(name);
                }
            }
            None
        }
        _ => None,
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
        let c = CargoEcosystem;
        assert_eq!(c.id(), ID);
        assert_eq!(Ecosystem::kind(&c), EcosystemKind::Package);
        assert_eq!(Ecosystem::languages(&c), &["rust"]);
    }

    #[test]
    fn legacy_locator_tag_is_rust() {
        assert_eq!(ExternalSourceLocator::ecosystem(&CargoEcosystem), "rust");
    }

    // --- Cargo.lock parser ---

    #[test]
    fn parse_cargo_lock_registry_only() {
        let lock = concat!(
            "version = 3\n\n",
            "[[package]]\n",
            "name = \"anyhow\"\n",
            "version = \"1.0.82\"\n",
            "source = \"registry+https://github.com/rust-lang/crates.io-index\"\n",
            "checksum = \"abc\"\n\n",
            "[[package]]\n",
            "name = \"workspace-crate\"\n",
            "version = \"0.1.0\"\n\n",
            "[[package]]\n",
            "name = \"tokio\"\n",
            "version = \"1.38.0\"\n",
            "source = \"registry+https://github.com/rust-lang/crates.io-index\"\n",
            "checksum = \"def\"\n\n",
            "[[package]]\n",
            "name = \"git-dep\"\n",
            "version = \"0.5.0\"\n",
            "source = \"git+https://github.com/example/crate.git#abc\"\n",
        );
        let entries = parse_cargo_lock(lock);
        assert_eq!(entries.len(), 2);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"anyhow"));
        assert!(names.contains(&"tokio"));
        assert!(!names.contains(&"workspace-crate"));
        assert!(!names.contains(&"git-dep"));
    }

    #[test]
    fn split_crate_dir_name_handles_hyphenated_names() {
        assert_eq!(split_crate_dir_name("tokio-1.38.0"),
            Some(("tokio".into(), "1.38.0".into())));
        assert_eq!(split_crate_dir_name("proc-macro2-1.0.91"),
            Some(("proc-macro2".into(), "1.0.91".into())));
        assert_eq!(split_crate_dir_name("tokio-util-0.7.9"),
            Some(("tokio-util".into(), "0.7.9".into())));
        assert_eq!(split_crate_dir_name("no-version"), None);
    }

    // --- path deps parser (migrated from manifest/cargo.rs tests) ---

    #[test]
    fn path_deps_inline_table_single_line() {
        let toml = r#"
[dependencies]
serde = "1"
core = { path = "../core" }
tokio = { version = "1", features = ["full"] }
"#;
        let paths = parse_cargo_path_dependencies(toml);
        assert_eq!(paths, vec!["core"]);
    }

    #[test]
    fn path_deps_multi_line_inline_table() {
        let toml = r#"
[dependencies]
shared = {
    path = "../shared",
    version = "0.1"
}
remote = { version = "1" }
"#;
        let paths = parse_cargo_path_dependencies(toml);
        assert_eq!(paths, vec!["shared"]);
    }

    #[test]
    fn path_deps_across_multiple_dep_sections() {
        let toml = r#"
[dependencies]
core = { path = "../core" }

[dev-dependencies]
testutil = { path = "../testutil" }

[build-dependencies]
builder = { path = "../builder" }
"#;
        let paths = parse_cargo_path_dependencies(toml);
        assert!(paths.contains(&"core".to_string()));
        assert!(paths.contains(&"testutil".to_string()));
        assert!(paths.contains(&"builder".to_string()));
    }

    #[test]
    fn path_deps_ignores_registry_entries() {
        let toml = r#"
[dependencies]
serde = "1"
tokio = { version = "1" }
anyhow = "1.0"
"#;
        let paths = parse_cargo_path_dependencies(toml);
        assert!(paths.is_empty());
    }

    // --- discovery integration (migrated from externals/rust_lang.rs tests) ---

    #[test]
    fn discover_cargo_roots_uses_lockfile() {
        let tmp = std::env::temp_dir().join("bw-test-cargo-lock");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("Cargo.toml"), "[workspace]\nmembers = []\n").unwrap();
        let lock = concat!(
            "version = 3\n\n",
            "[[package]]\n",
            "name = \"serde\"\n",
            "version = \"1.0.200\"\n",
            "source = \"registry+https://github.com/rust-lang/crates.io-index\"\n",
            "checksum = \"abc\"\n",
        );
        std::fs::write(tmp.join("Cargo.lock"), lock).unwrap();

        let fake_home = tmp.join("fake_cargo_home");
        let serde_src = fake_home
            .join("registry").join("src").join("index-abc")
            .join("serde-1.0.200").join("src");
        std::fs::create_dir_all(&serde_src).unwrap();
        std::fs::write(serde_src.join("lib.rs"), "pub trait Serialize {}").unwrap();

        std::env::set_var("CARGO_HOME", fake_home.to_str().unwrap());
        let roots = discover_cargo_roots(&tmp);
        std::env::remove_var("CARGO_HOME");

        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].module_path, "serde");
        assert_eq!(roots[0].version, "1.0.200");

        let walked = walk_cargo_root(&roots[0]);
        assert_eq!(walked.len(), 1);
        assert!(walked[0].relative_path.starts_with("ext:rust:serde/"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn discover_cargo_roots_empty_without_cargo_toml() {
        let tmp = std::env::temp_dir().join("bw-test-cargo-no-toml");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let roots = discover_cargo_roots(&tmp);
        assert!(roots.is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[allow(dead_code)]
    fn _ensure_shared_locator_typed() -> Arc<dyn ExternalSourceLocator> {
        shared_locator()
    }

    // -----------------------------------------------------------------
    // R3 — reachability-based crate entry resolution
    // -----------------------------------------------------------------

    fn mkdep(root: PathBuf, name: &str, version: &str) -> ExternalDepRoot {
        ExternalDepRoot {
            module_path: name.to_string(),
            version: version.to_string(),
            root,
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: Vec::new(),
        }
    }

    #[test]
    fn extract_mod_decls_matches_pub_and_bare() {
        let src = r#"
pub mod a;
mod b;
pub(crate) mod c;
pub(super) mod d;
mod e; // inline comment
use foo::bar;
pub mod inline { pub fn f() {} }  // inline body — has no ;
mod ok_end;
"#;
        let decls = extract_rust_mod_decls(src);
        assert!(decls.contains(&"a".to_string()));
        assert!(decls.contains(&"b".to_string()));
        assert!(decls.contains(&"c".to_string()));
        assert!(decls.contains(&"d".to_string()));
        assert!(decls.contains(&"e".to_string()));
        assert!(decls.contains(&"ok_end".to_string()));
        assert!(!decls.contains(&"inline".to_string()));
    }

    #[test]
    fn resolve_crate_entry_follows_mod_tree() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("serde-1.0.200");
        let src = root.join("src");
        std::fs::create_dir_all(src.join("de")).unwrap();
        std::fs::write(
            src.join("lib.rs"),
            "pub mod ser;\nmod de;\n",
        ).unwrap();
        std::fs::write(src.join("ser.rs"), "pub trait Serialize {}\n").unwrap();
        std::fs::write(src.join("de").join("mod.rs"), "pub mod inner;\n").unwrap();
        std::fs::write(src.join("de").join("inner.rs"), "pub struct Inner;\n").unwrap();

        let dep = mkdep(root.clone(), "serde", "1.0.200");
        let files = CargoEcosystem.resolve_import(&dep, "serde", &["Serialize"]);
        assert_eq!(files.len(), 4, "got: {:?}", files);
        let paths: std::collections::HashSet<_> = files
            .iter()
            .map(|f| f.absolute_path.clone())
            .collect();
        assert!(paths.contains(&src.join("lib.rs")));
        assert!(paths.contains(&src.join("ser.rs")));
        assert!(paths.contains(&src.join("de").join("mod.rs")));
        assert!(paths.contains(&src.join("de").join("inner.rs")));
        for f in &files {
            assert!(f.relative_path.starts_with("ext:rust:serde/"));
            assert_eq!(f.language, "rust");
        }
    }

    #[test]
    fn resolve_crate_entry_falls_back_to_main_rs() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("bin-only-0.1.0");
        let src = root.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("main.rs"), "fn main() {}\n").unwrap();

        let dep = mkdep(root.clone(), "bin-only", "0.1.0");
        let files = CargoEcosystem.resolve_import(&dep, "bin-only", &[]);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].absolute_path, src.join("main.rs"));
    }

    #[test]
    fn resolve_crate_entry_empty_without_src_entry() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("no-entry-0.1.0");
        std::fs::create_dir_all(&root).unwrap();

        let dep = mkdep(root, "no-entry", "0.1.0");
        assert!(CargoEcosystem.resolve_import(&dep, "no-entry", &[]).is_empty());
    }

    #[test]
    fn resolve_rust_mod_path_handles_both_layouts() {
        let tmp = tempfile::TempDir::new().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        let lib = src.join("lib.rs");
        std::fs::write(&lib, "").unwrap();

        // sibling .rs layout
        let a = src.join("a.rs");
        std::fs::write(&a, "").unwrap();
        assert_eq!(resolve_rust_mod_path(&lib, "a"), Some(a));

        // directory/mod.rs layout
        let sub_mod = src.join("sub").join("mod.rs");
        std::fs::write(&sub_mod, "").unwrap();
        assert_eq!(resolve_rust_mod_path(&lib, "sub"), Some(sub_mod));

        // missing module
        assert_eq!(resolve_rust_mod_path(&lib, "missing"), None);
    }

    #[test]
    fn rust_header_scanner_captures_top_level_items() {
        let src = r#"
pub struct Foo {
    x: i32,
}

pub enum Status { Ok, Err }

pub trait Service {
    fn call(&self) -> Result<(), ()>;
}

pub fn top_level_fn() -> i32 { 0 }

pub const MAX: usize = 10;

pub static NAME: &str = "x";

pub type Alias = Foo;

macro_rules! my_macro { () => {}; }

impl Foo {
    pub fn new() -> Self { Foo { x: 0 } }
    pub fn helper(&self) {}
}
"#;
        let names = scan_rust_header(src);
        assert!(names.contains(&"Foo".to_string()), "{names:?}");
        assert!(names.contains(&"Status".to_string()), "{names:?}");
        assert!(names.contains(&"Service".to_string()), "{names:?}");
        assert!(names.contains(&"top_level_fn".to_string()), "{names:?}");
        assert!(names.contains(&"MAX".to_string()), "{names:?}");
        assert!(names.contains(&"NAME".to_string()), "{names:?}");
        assert!(names.contains(&"Alias".to_string()), "{names:?}");
        assert!(names.contains(&"new".to_string()), "{names:?}");
        assert!(names.contains(&"Foo::new".to_string()), "{names:?}");
        assert!(names.contains(&"Foo::helper".to_string()), "{names:?}");
    }

    #[test]
    fn rust_build_symbol_index_empty_returns_empty() {
        assert!(build_cargo_symbol_index(&[]).is_empty());
    }
}
