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

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
    SymbolLocationIndex,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("cargo");

const MANIFESTS: &[ManifestSpec] = &[];
const LANGUAGES: &[&str] = &["rust"];
pub(super) const LEGACY_ECOSYSTEM_TAG: &str = "rust";

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
        // Project deps via `Cargo.toml`. The Rust toolchain (prelude, core,
        // std) belongs to `rust-stdlib`; cargo only resolves declared crate
        // deps. A `.rs` file with no `Cargo.toml` has nothing for this
        // ecosystem to point at, so dropping the LanguagePresent shotgun
        // is correct per the trait doc.
        EcosystemActivation::ManifestMatch
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

mod discovery;
mod manifest;
mod reachability;
mod symbol_index;

pub use manifest::{parse_cargo_dependencies, parse_cargo_path_dependencies, CargoManifest};
pub(crate) use symbol_index::build_cargo_symbol_index;

use discovery::discover_cargo_roots;
use reachability::{lib_entry_from_manifest, resolve_crate_entry};

// ---------------------------------------------------------------------------
// Walk
// ---------------------------------------------------------------------------

pub(crate) fn walk_cargo_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    // Honor `[lib] path = "..."` first — `tree-sitter` and similar
    // C-with-Rust-bindings crates put their entry under
    // `binding_rust/lib.rs` (or similar), not `src/`. If we walk only
    // `src/` for those, we get zero `.rs` files and the
    // SymbolLocationIndex never sees their public surface.
    let manifest_entry = lib_entry_from_manifest(&dep.root);
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(entry) = manifest_entry.as_ref() {
        if let Some(parent) = entry.parent() {
            roots.push(parent.to_path_buf());
        }
    }
    let src = dep.root.join("src");
    if src.is_dir() && !roots.iter().any(|r| r == &src) {
        roots.push(src);
    }
    if roots.is_empty() {
        roots.push(dep.root.clone());
    }
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for r in &roots {
        if !seen.insert(r.clone()) { continue }
        walk_dir_bounded(r, &dep.root, dep, &mut out, 0);
    }
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

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
