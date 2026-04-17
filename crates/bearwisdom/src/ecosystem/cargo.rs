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

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
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
                });
            }
        }
    }

    debug!("Rust: discovered {} external crate roots", roots.len());
    roots
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
}
