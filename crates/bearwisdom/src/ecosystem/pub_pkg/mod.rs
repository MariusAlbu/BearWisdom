// =============================================================================
// ecosystem/pub_pkg — Dart Pub ecosystem
//
// Phase 2 + 3: consolidates `indexer/externals/dart.rs` +
// `indexer/manifest/pubspec.rs`. Two resolution strategies:
//   1. `.dart_tool/package_config.json` (Dart 2.5+) — exact paths.
//   2. `pubspec.lock` + pub cache walk (`~/.pub-cache/hosted/pub.dev/`).
//
// Module named `pub_pkg` because `pub` is a Rust keyword.
// =============================================================================

use std::path::Path;
use std::sync::Arc;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
    SymbolLocationIndex,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("pub");

const MANIFESTS: &[ManifestSpec] = &[];
const LANGUAGES: &[&str] = &["dart"];
pub(super) const LEGACY_ECOSYSTEM_TAG: &str = "dart";

pub struct PubEcosystem;

// ---------------------------------------------------------------------------
// Ecosystem trait impl
// ---------------------------------------------------------------------------

impl Ecosystem for PubEcosystem {
    fn id(&self) -> EcosystemId {
        ID
    }
    fn kind(&self) -> EcosystemKind {
        EcosystemKind::Package
    }
    fn languages(&self) -> &'static [&'static str] {
        LANGUAGES
    }
    fn manifest_specs(&self) -> &'static [ManifestSpec] {
        MANIFESTS
    }

    fn workspace_package_files(&self) -> &'static [(&'static str, &'static str)] {
        // Kind label "dart" matches the legacy ecosystem tag and the kind
        // string used by the Pub manifest reader (see ManifestKind::Pubspec
        // → "dart" in stage_discover::manifest_kind_to_ecosystem).
        &[("pubspec.yaml", "dart")]
    }

    fn pruned_dir_names(&self) -> &'static [&'static str] {
        &[".dart_tool", ".pub-cache"]
    }

    fn activation(&self) -> EcosystemActivation {
        // Project deps via `pubspec.yaml`. A bare directory of `.dart`
        // files with no manifest can't be resolved against external
        // pub.dev coordinates, so dropping the LanguagePresent shotgun
        // is correct per the trait doc.
        EcosystemActivation::ManifestMatch
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_dart_externals(ctx.project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_dart_root(dep)
    }

    fn supports_reachability(&self) -> bool {
        true
    }

    fn resolve_import(
        &self,
        dep: &ExternalDepRoot,
        _package: &str,
        _symbols: &[&str],
    ) -> Vec<WalkedFile> {
        resolve_dart_package_entry(dep)
    }

    fn resolve_symbol(&self, dep: &ExternalDepRoot, _fqn: &str) -> Vec<WalkedFile> {
        resolve_dart_package_entry(dep)
    }

    fn build_symbol_index(&self, dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
        build_dart_symbol_index(dep_roots)
    }

    fn uses_demand_driven_parse(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// Legacy ExternalSourceLocator impl
// ---------------------------------------------------------------------------

impl ExternalSourceLocator for PubEcosystem {
    fn ecosystem(&self) -> &'static str {
        LEGACY_ECOSYSTEM_TAG
    }
    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_dart_externals(project_root)
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_dart_root(dep)
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<PubEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(PubEcosystem)).clone()
}

mod discovery;
mod manifest;
mod reachability;
mod symbol_index;
mod walk;

pub use discovery::{discover_dart_externals, find_pub_cache, parse_pubspec_lock};
pub use manifest::{parse_pubspec_deps, PubspecManifest};
pub(crate) use manifest::parse_pubspec_name;
pub(crate) use symbol_index::build_dart_symbol_index;
pub use walk::walk_dart_root;

use reachability::resolve_dart_package_entry;

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
