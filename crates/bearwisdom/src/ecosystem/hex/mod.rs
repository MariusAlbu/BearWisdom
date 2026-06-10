// =============================================================================
// ecosystem/hex.rs — Hex / BEAM ecosystem (Elixir + Erlang + Gleam)
//
// Three languages share the Hex package manager (hex.pm) and the BEAM
// runtime. They differ in install layout:
//
//   - Elixir   — mix places deps under `<project>/deps/<name>/` (project-local)
//   - Erlang   — rebar3 compiles into `<project>/_build/default/lib/<name>/`,
//                OR hex tarballs sit at `~/.hex/packages/hexpm/<name>-<ver>.tar`
//                (shared with Elixir); we extract and cache
//   - Gleam    — gleam fetches into `<project>/build/packages/<name>/`
//
// HexEcosystem runs all three discoveries and unions the roots. The unified
// walker detects source language by extension (.ex/.exs/.erl/.hrl/.gleam)
// so Erlang source inside an Elixir hex dep (e.g. cowboy ships .erl) parses
// correctly.
//
// Before this refactor:
//   indexer/externals/elixir.rs — ElixirExternalsLocator  (~360 LOC)
//   indexer/externals/erlang.rs — ErlangExternalsLocator  (~739 LOC)
//   indexer/externals/gleam.rs  — GleamExternalsLocator   (~93 LOC)
// =============================================================================

use std::path::Path;
use std::sync::Arc;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
    SymbolLocationIndex,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("hex");

const MANIFESTS: &[ManifestSpec] = &[];
const LANGUAGES: &[&str] = &["elixir", "erlang", "gleam"];

// Legacy ecosystem tags written into `ExternalDepRoot::ecosystem` so the
// existing indexer dispatch (keyed on root.ecosystem matching
// locator.ecosystem()) still works. The legacy single-locator-per-ecosystem
// string was "elixir"/"erlang"/"gleam"; we now report "hex" for all three
// since a single HexEcosystem locator handles every walk_root dispatch.
pub(super) const LEGACY_ECOSYSTEM_TAG: &str = "hex";

pub struct HexEcosystem;

// ---------------------------------------------------------------------------
// Ecosystem trait impl (new)
// ---------------------------------------------------------------------------

impl Ecosystem for HexEcosystem {
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
        // Hex covers the Erlang/Elixir/Gleam triumvirate. Each tool brings
        // its own manifest filename; map them to distinct kinds so users
        // can tell them apart in queries.
        &[
            ("mix.exs", "elixir"),
            ("rebar.config", "erlang"),
            ("gleam.toml", "gleam"),
        ]
    }

    fn pruned_dir_names(&self) -> &'static [&'static str] {
        &["_build", "deps"]
    }

    fn activation(&self) -> EcosystemActivation {
        // Project deps via mix.exs / gleam.toml. A bare directory of
        // `.ex`/`.erl`/`.gleam` files without a manifest can't be
        // resolved against external Hex coordinates, so dropping the
        // LanguagePresent shotgun is correct per the trait doc.
        EcosystemActivation::ManifestMatch
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_hex_roots(ctx.project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_hex_root(dep)
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
        walk_hex_narrowed(dep)
    }

    fn resolve_symbol(&self, dep: &ExternalDepRoot, _fqn: &str) -> Vec<WalkedFile> {
        walk_hex_narrowed(dep)
    }

    fn build_symbol_index(&self, dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
        build_hex_symbol_index(dep_roots)
    }

    fn uses_demand_driven_parse(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// Legacy ExternalSourceLocator impl
// ---------------------------------------------------------------------------

impl ExternalSourceLocator for HexEcosystem {
    fn ecosystem(&self) -> &'static str {
        LEGACY_ECOSYSTEM_TAG
    }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_hex_roots(project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_hex_root(dep)
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<HexEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(HexEcosystem)).clone()
}

mod discovery;
mod reachability;
mod symbol_index;
mod walk;

pub use discovery::{parse_rebar_deps, parse_rebar_lock};
pub(crate) use symbol_index::build_hex_symbol_index;

use discovery::discover_hex_roots;
use reachability::walk_hex_narrowed;
use walk::walk_hex_root;

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
