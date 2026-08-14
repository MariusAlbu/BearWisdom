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
/// file-path candidate set (grown once per `ingest` batch), the workspace's
/// `go.mod` module path, and the declared workspace packages (name → root
/// directory), all set once from `ProjectContext`.
#[derive(Default)]
pub(crate) struct Context {
    pub(crate) file_paths: Vec<String>,
    pub(crate) go_module_path: Option<String>,
    pub(crate) workspace_packages: Vec<(String, String)>,
}

impl Context {
    /// Reset the manifest-derived resolver signals from a fresh
    /// `ProjectContext`: the go.mod module path and the declared workspace
    /// packages as (name → root directory) pairs, roots normalized without a
    /// trailing slash.
    pub(crate) fn snapshot_manifests(
        &mut self,
        ctx: &crate::indexer::project_context::ProjectContext,
    ) {
        self.go_module_path = ctx
            .manifests
            .get(&crate::ecosystem::manifest::ManifestKind::GoMod)
            .and_then(|m| m.module_path.clone());
        self.workspace_packages = ctx
            .workspace_pkg_by_declared_name
            .iter()
            .filter_map(|(name, id)| {
                let root = ctx.workspace_pkg_paths.get(id)?;
                Some((name.clone(), root.trim_end_matches('/').to_string()))
            })
            .collect();
    }
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
/// registered `ModuleResolver`. `dart_self_package` is `source_file`'s OWNING
/// package's own declared name — recovered from `package_id` against
/// `workspace_pkg_by_declared_name` — not one project-wide guess, since a
/// workspace can hold several pubspec-declared Dart packages.
pub(crate) fn resolve_via_module_resolver(
    language: &str,
    source_file: &str,
    spec: &str,
    package_id: Option<i64>,
    workspace_pkg_by_declared_name: &FxHashMap<String, i64>,
    go_module_path: Option<&str>,
    workspace_packages: &[(String, String)],
    file_paths: &[String],
) -> Option<String> {
    let dart_self_package = package_id.and_then(|pid| {
        workspace_pkg_by_declared_name
            .iter()
            .find(|(_, &id)| id == pid)
            .map(|(name, _)| name.as_str())
    });
    let resolvers = crate::indexer::module_resolution::all_resolvers_with_workspace(
        go_module_path,
        dart_self_package,
        workspace_packages.to_vec(),
    );
    let paths: Vec<&str> = file_paths.iter().map(String::as_str).collect();
    crate::indexer::module_resolution::resolve_module_to_file(
        language, spec, source_file, &paths, &resolvers,
    )
}

#[cfg(test)]
#[path = "module_specifier_tests.rs"]
mod tests;
