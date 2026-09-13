// =============================================================================
// engine/module_specifier — Compilation's ModuleResolver dispatch
//
// The logic behind `SymbolLookup::resolve_module_via_language_resolver`:
// collects a project's internal file paths and drives
// `indexer::module_resolution`'s per-language resolver registry against them.
// =============================================================================

use rustc_hash::FxHashMap;

use crate::types::ParsedFile;

/// `Compilation`'s state for the `ModuleResolver` fallback: the internal
/// file-path candidate set (grown once per `ingest` batch), opaque
/// ecosystem-owned resolver inputs, and declared workspace packages (name →
/// root directory), all set once from `ProjectContext`.
#[derive(Default)]
pub(crate) struct Context {
    pub(crate) file_paths: Vec<String>,
    pub(crate) resolver_inputs: crate::ecosystem::module_specifier::ResolverInputs,
    pub(crate) workspace_packages: Vec<(String, String)>,
    /// Workspace package id → its manifest's project-relative entry
    /// candidates, in declared priority.
    pub(crate) workspace_entries: FxHashMap<i64, Vec<String>>,
    /// Workspace package id → the one name that package declared for itself.
    pub(crate) pkg_declared_name: FxHashMap<i64, String>,
    /// Workspace package id → the project-relative directory its module
    /// sub-paths are rooted at, for packages that declare one.
    pub(crate) pkg_source_root: FxHashMap<i64, String>,
}

impl Context {
    /// Reset the manifest-derived resolver signals from a fresh
    /// `ProjectContext`: ecosystem-owned resolver inputs and declared workspace
    /// packages as (name → root directory) pairs, roots normalized without a
    /// trailing slash.
    pub(crate) fn snapshot_manifests(
        &mut self,
        ctx: &crate::indexer::project_context::ProjectContext,
    ) {
        self.resolver_inputs =
            crate::ecosystem::module_specifier::ResolverInputs::from_project_context(ctx);
        self.workspace_packages = ctx
            .workspace
            .by_declared_name
            .iter()
            .filter_map(|(name, id)| {
                let root = ctx.workspace.paths.get(id)?;
                Some((name.clone(), root.trim_end_matches('/').to_string()))
            })
            .collect();
        self.workspace_entries = ctx
            .workspace
            .entries
            .iter()
            .map(|(id, entries)| (*id, entries.clone()))
            .collect();
        self.pkg_declared_name = clone_by_id(&ctx.workspace.declared_name);
        self.pkg_source_root = clone_by_id(&ctx.workspace.source_roots);
    }
}

/// Re-key a package-id-keyed project map into the resolver's own map type.
fn clone_by_id(source: &std::collections::HashMap<i64, String>) -> FxHashMap<i64, String> {
    source
        .iter()
        .map(|(id, value)| (*id, value.clone()))
        .collect()
}

/// The internal (non-`ext:`) file paths from a parsed batch — the candidate
/// set a `ModuleResolver` suffix-matches a specifier against.
pub(crate) fn internal_file_paths(parsed: &[ParsedFile]) -> Vec<String> {
    parsed
        .iter()
        .map(|pf| pf.path.as_str())
        .filter(|p| !p.starts_with("ext:"))
        .map(str::to_string)
        .collect()
}

/// Resolve `spec` (as written in `source_file`'s import) through `language`'s
/// registered `ModuleResolver`. `source_package_name` is the importing file's
/// own declared package name — read from `package_id` rather than guessed
/// project-wide, since a workspace can contain several package roots. It is
/// the package's CANONICAL spelling, so a resolver that reconstructs the
/// source's own specifier gets the name its manifest declared rather than an
/// ecosystem alias of it.
pub(crate) fn resolve_via_module_resolver(
    language: &str,
    source_file: &str,
    spec: &str,
    package_id: Option<i64>,
    pkg_declared_name: &FxHashMap<i64, String>,
    resolver_inputs: &crate::ecosystem::module_specifier::ResolverInputs,
    workspace_packages: &[(String, String)],
    file_paths: &[String],
) -> Option<String> {
    let source_package_name =
        package_id.and_then(|pid| pkg_declared_name.get(&pid).map(String::as_str));
    let resolvers = crate::ecosystem::module_specifier::language_resolvers(
        resolver_inputs,
        source_package_name,
        workspace_packages.to_vec(),
    );
    let paths: Vec<&str> = file_paths.iter().map(String::as_str).collect();
    crate::indexer::module_resolution::resolve_module_to_file(
        language,
        spec,
        source_file,
        &paths,
        &resolvers,
    )
}

#[cfg(test)]
#[path = "module_specifier_tests.rs"]
mod tests;
