// =============================================================================
// ecosystem/go_mod.rs — Go module ecosystem
//
// Phase 2 + 3: consolidates `indexer/externals/go.rs` +
// `indexer/manifest/go_mod.rs`. Go's module cache lives at
// `$GOMODCACHE/{escaped_module_path}@{version}`; indirect deps are walked
// only when a lightweight source scan confirms user code imports them.
// =============================================================================

use std::path::Path;
use std::sync::Arc;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
    SymbolLocationIndex,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("go-mod");

const MANIFESTS: &[ManifestSpec] = &[];
const LANGUAGES: &[&str] = &["go"];
pub(super) const LEGACY_ECOSYSTEM_TAG: &str = "go";

pub struct GoModEcosystem;

impl Ecosystem for GoModEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Package }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }
    fn manifest_specs(&self) -> &'static [ManifestSpec] { MANIFESTS }

    fn workspace_package_files(&self) -> &'static [(&'static str, &'static str)] {
        &[("go.mod", "go")]
    }

    fn pruned_dir_names(&self) -> &'static [&'static str] {
        // Go's vendor convention; Go modules cache lives outside the project.
        &["vendor"]
    }

    fn activation(&self) -> EcosystemActivation {
        // Project deps via `go.mod`. The Go module cache layout is
        // version-pinned per dep; without a `go.mod` there's nothing to
        // pin against. Dropping the LanguagePresent shotgun is correct
        // per the trait doc.
        EcosystemActivation::ManifestMatch
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_go_externals(ctx.project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_go_root(dep)
    }

    fn supports_reachability(&self) -> bool { true }

    fn resolve_import(
        &self,
        dep: &ExternalDepRoot,
        _package: &str,
        _symbols: &[&str],
    ) -> Vec<WalkedFile> {
        resolve_go_requested_packages(dep)
    }

    fn resolve_symbol(
        &self,
        dep: &ExternalDepRoot,
        _fqn: &str,
    ) -> Vec<WalkedFile> {
        resolve_go_requested_packages(dep)
    }

    fn build_symbol_index(
        &self,
        dep_roots: &[ExternalDepRoot],
    ) -> SymbolLocationIndex {
        build_go_symbol_index(dep_roots)
    }

    fn uses_demand_driven_parse(&self) -> bool { true }
}

impl ExternalSourceLocator for GoModEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_go_externals(project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_go_root(dep)
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<GoModEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(GoModEcosystem)).clone()
}

mod discovery;
mod manifest;
mod reachability;
mod symbol_index;

pub use discovery::{discover_go_externals, gomodcache_root};
pub use manifest::{find_go_mod, parse_go_mod, GoModData, GoModDep, GoModManifest};
pub(crate) use symbol_index::build_go_symbol_index;

use reachability::resolve_go_requested_packages;

// ---------------------------------------------------------------------------
// Walk
// ---------------------------------------------------------------------------

pub(crate) fn walk_go_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_dir_bounded(&dep.root, &dep.root, dep, &mut out, 0);
    out
}

fn walk_dir_bounded(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= MAX_WALK_DEPTH { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else { continue };
        let path = entry.path();
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "vendor" | "testdata" | ".git" | "_examples") { continue }
            }
            walk_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".go") { continue }
            if name.ends_with("_test.go") { continue }
            if !super::go_platform::file_matches_host(name) { continue }
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

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
