//! Ecosystem-owned policy used while materializing external source files.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::type_checker::core::types::TypeArena;
use crate::types::{ExtractedRef, ParsedFile};
use crate::walker::WalkedFile;
use crate::{ecosystem::externals::ExternalDepRoot, indexer::demand::DemandSet};

#[path = "npm/external_policy.rs"]
mod npm;
#[path = "npm/external_demand_policy.rs"]
mod npm_demand;
#[path = "posix_headers/external_policy.rs"]
mod posix_headers;

type PostProcess = fn(&mut ParsedFile, &TypeArena) -> bool;
type CollectRelativeSupertypes =
    fn(&str, &Path, &[ExtractedRef], &mut HashSet<PathBuf>, &mut Vec<PathBuf>) -> bool;
type ResolveRelativeModule = fn(&str, &Path, &str) -> Option<PathBuf>;
type SecondaryScan = fn(&Path, &[WalkedFile]) -> Vec<WalkedFile>;
type AmbientGlobalPackages = fn(&[ExternalDepRoot]) -> HashSet<String>;
type ExtensionlessSourceLanguage = fn(&str) -> Option<&'static str>;
type ExternalDemand =
    for<'a> fn(&str, &'a DemandSet, &HashSet<String>) -> ExternalDemandDecision<'a>;

/// Normalized provider decision for parsing one external file.
pub(crate) enum ExternalDemandDecision<'a> {
    /// This provider does not own the file's virtual-path layout.
    Decline,
    /// The provider owns the file and requires permissive parsing.
    Full,
    /// The provider owns the file and supplies its observed symbol demand.
    Filter(&'a HashSet<String>),
}

struct Adapter {
    post_process: PostProcess,
    collect_relative_supertypes: CollectRelativeSupertypes,
    resolve_relative_module: ResolveRelativeModule,
    secondary_scan: SecondaryScan,
    ambient_global_packages: AmbientGlobalPackages,
    external_demand: ExternalDemand,
}

const ADAPTERS: &[Adapter] = &[Adapter {
    post_process: npm::post_process,
    collect_relative_supertypes: npm::collect_relative_supertypes,
    resolve_relative_module: npm::resolve_relative_module,
    secondary_scan: npm::secondary_scan,
    ambient_global_packages: npm_demand::ambient_global_packages,
    external_demand: npm_demand::external_demand,
}];

const EXTENSIONLESS_SOURCE_ADAPTERS: &[ExtensionlessSourceLanguage] =
    &[posix_headers::extensionless_source_language];

pub(crate) fn post_process(parsed: &mut ParsedFile, arena: &TypeArena) {
    let _ = ADAPTERS
        .iter()
        .find(|adapter| (adapter.post_process)(parsed, arena));
}

pub(crate) fn collect_relative_supertypes(
    language: &str,
    importer: &Path,
    refs: &[ExtractedRef],
    seen: &mut HashSet<PathBuf>,
    out: &mut Vec<PathBuf>,
) {
    let _ = ADAPTERS
        .iter()
        .find(|adapter| (adapter.collect_relative_supertypes)(language, importer, refs, seen, out));
}

pub(crate) fn resolve_relative_module(
    language: &str,
    directory: &Path,
    specifier: &str,
) -> Option<PathBuf> {
    ADAPTERS
        .iter()
        .find_map(|adapter| (adapter.resolve_relative_module)(language, directory, specifier))
}

/// Run every provider's additive secondary source scan. Providers own their
/// import syntax and source-layout rules; this registry only combines output.
pub(crate) fn scan_secondary_sources(
    project_root: &Path,
    primary: &[WalkedFile],
) -> Vec<WalkedFile> {
    ADAPTERS
        .iter()
        .flat_map(|adapter| (adapter.secondary_scan)(project_root, primary))
        .collect()
}

/// Let providers identify extensionless source files that the language
/// registry cannot classify by filename alone.
pub(crate) fn extensionless_source_language(file_name: &str) -> Option<&'static str> {
    EXTENSIONLESS_SOURCE_ADAPTERS
        .iter()
        .find_map(|adapter| adapter(file_name))
}

/// Combine each provider's declaration-global package inventory. Providers
/// decide both the on-disk probe and the package layouts it applies to.
pub(crate) fn ambient_global_packages(roots: &[ExternalDepRoot]) -> HashSet<String> {
    ADAPTERS
        .iter()
        .flat_map(|adapter| (adapter.ambient_global_packages)(roots))
        .collect()
}

/// Return the first provider decision that claims `relative_path`.
pub(crate) fn external_demand<'a>(
    relative_path: &str,
    demand: &'a DemandSet,
    ambient_global_packages: &HashSet<String>,
) -> ExternalDemandDecision<'a> {
    ADAPTERS
        .iter()
        .map(|adapter| (adapter.external_demand)(relative_path, demand, ambient_global_packages))
        .find(|decision| !matches!(decision, ExternalDemandDecision::Decline))
        .unwrap_or(ExternalDemandDecision::Decline)
}

#[cfg(test)]
#[path = "external_policy_tests.rs"]
mod tests;
