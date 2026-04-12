// =============================================================================
// indexer/manifest/mod.rs — ManifestReader trait and dispatch
//
// Each language ecosystem has a dedicated reader that locates manifest files
// under the project root and extracts a normalized `ManifestData` from them.
//
// `read_all_manifests` runs every registered reader and returns a map keyed
// by `ManifestKind`. `build_project_context` calls this once and back-fills
// the per-language fields on `ProjectContext` for backward compatibility.
// =============================================================================

pub mod cargo;
pub mod composer;
pub mod description;
pub mod gemfile;
pub mod go_mod;
pub mod gradle;
pub mod maven;
pub mod mix;
pub mod npm;
pub mod nuget;
pub mod sbt;
pub mod opam;
pub mod gleam;
pub mod zig_zon;
pub mod clojure;
pub mod rockspec;
pub mod pubspec;
pub mod pyproject;
pub mod swift_pm;

use std::collections::{HashMap, HashSet};
use std::path::Path;

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

/// Normalized set of dependency names for a given ecosystem.
pub type DependencySet = HashSet<String>;

/// Which ecosystem produced a `ManifestData` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ManifestKind {
    Npm,
    Cargo,
    NuGet,
    GoMod,
    PyProject,
    Gradle,
    Maven,
    Gemfile,
    Composer,
    SwiftPM,
    Pubspec,
    Mix,
    Description,
    Sbt,
    Opam,
    Gleam,
    ZigZon,
    Clojure,
    Rockspec,
}

/// Normalized data extracted from a project manifest.
#[derive(Debug, Clone, Default)]
pub struct ManifestData {
    /// Dependency names (package/crate/gem/etc names, no versions).
    pub dependencies: DependencySet,
    /// Go: module path from `go.mod`. Swift: package name. Others: `None`.
    pub module_path: Option<String>,
    /// .NET: combined SDK implicit usings + `global using` statements.
    pub global_usings: Vec<String>,
    /// .NET: most capable SDK type string (`"web"`, `"base"`, etc.).
    pub sdk_type: Option<String>,
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Reads one ecosystem's manifest files and returns normalized data.
///
/// Implementations are stateless — construct once, call `read` many times.
pub trait ManifestReader: Send + Sync {
    fn kind(&self) -> ManifestKind;

    /// Scan `project_root` for manifest files.
    ///
    /// Returns `None` when no manifest for this ecosystem is found.
    /// Returns `Some(ManifestData::default())` when a file exists but is empty
    /// or unparseable — callers can still detect the ecosystem's presence.
    fn read(&self, project_root: &Path) -> Option<ManifestData>;
}

// ---------------------------------------------------------------------------
// Registry & dispatch
// ---------------------------------------------------------------------------

/// All registered `ManifestReader` implementations, in stable order.
fn all_readers() -> Vec<Box<dyn ManifestReader>> {
    vec![
        Box::new(npm::NpmManifest),
        Box::new(cargo::CargoManifest),
        Box::new(nuget::NuGetManifest),
        Box::new(go_mod::GoModManifest),
        Box::new(pyproject::PyProjectManifest),
        Box::new(gradle::GradleManifest),
        Box::new(maven::MavenManifest),
        Box::new(gemfile::GemfileManifest),
        Box::new(composer::ComposerManifest),
        Box::new(swift_pm::SwiftPMManifest),
        Box::new(pubspec::PubspecManifest),
        Box::new(mix::MixManifest),
        Box::new(sbt::SbtManifest),
        Box::new(opam::OpamManifest),
        Box::new(gleam::GleamManifest),
        Box::new(zig_zon::ZigZonManifest),
        Box::new(clojure::ClojureManifest),
        Box::new(rockspec::RockspecManifest),
    ]
}

/// Run every registered reader against `project_root`.
///
/// Returns only the ecosystems for which a manifest was actually found.
pub fn read_all_manifests(project_root: &Path) -> HashMap<ManifestKind, ManifestData> {
    let mut result = HashMap::new();
    for reader in all_readers() {
        if let Some(data) = reader.read(project_root) {
            result.insert(reader.kind(), data);
        }
    }
    result
}
