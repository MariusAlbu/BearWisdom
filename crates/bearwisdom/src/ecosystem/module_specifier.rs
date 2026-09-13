// =============================================================================
// ecosystem/module_specifier — ecosystem-owned module-entry adapters
//
// The resolver calls this registry without knowing which ecosystem recognizes
// a virtual path or source specifier. Each adapter owns its schemes, package
// grammar, relative-path rules, and canonical module-entry keys.
// =============================================================================

type EntryAliases = fn(&str) -> Vec<String>;
type RelativeEntryKey = fn(&str, &str) -> Option<String>;
type PackageEntryKey = fn(&str) -> Option<String>;

/// Opaque ecosystem-owned inputs for language module resolvers.  The engine
/// stores and forwards this contract without selecting manifests or knowing
/// individual ecosystem spelling rules.
#[derive(Debug, Clone, Default)]
pub(crate) struct ResolverInputs {
    go_module_path: Option<String>,
}

impl ResolverInputs {
    pub(crate) fn from_project_context(
        context: &crate::indexer::project_context::ProjectContext,
    ) -> Self {
        Self {
            go_module_path: super::go_mod::module_specifier::project_module_path(context),
        }
    }
}

struct Adapter {
    entry_aliases: EntryAliases,
    relative_entry_key: RelativeEntryKey,
}

const ADAPTERS: &[Adapter] = &[
    Adapter {
        entry_aliases: super::npm::module_specifier::entry_aliases,
        relative_entry_key: super::npm::module_specifier::relative_entry_key,
    },
    Adapter {
        entry_aliases: super::pub_pkg::module_specifier::entry_aliases,
        relative_entry_key: super::pub_pkg::module_specifier::relative_entry_key,
    },
];

const PACKAGE_ENTRY_ADAPTERS: &[PackageEntryKey] = &[
    super::npm::module_specifier::package_entry_key,
    super::rubygems::module_specifier::package_entry_key,
    super::pub_pkg::module_specifier::package_entry_key,
];

pub(crate) fn entry_aliases(path: &str) -> Vec<String> {
    ADAPTERS
        .iter()
        .flat_map(|adapter| (adapter.entry_aliases)(path))
        .collect()
}

pub(crate) fn relative_entry_key(source_file: &str, specifier: &str) -> Option<String> {
    ADAPTERS
        .iter()
        .find_map(|adapter| (adapter.relative_entry_key)(source_file, specifier))
}

/// Return an exact module-entry key contributed by an ecosystem adapter.
/// Unknown virtual-path schemes fail closed: storage envelopes and package
/// segment grammar are never interpreted by generic resolver code.
pub(crate) fn package_entry_key(path: &str) -> Option<String> {
    PACKAGE_ENTRY_ADAPTERS
        .iter()
        .find_map(|adapter| adapter(path))
}

/// Match an import spelling to a declared workspace package and split off the
/// part of the spelling below the package name. The remainder is empty when
/// the spelling is the package name itself; the longest declared name wins, so
/// a package whose name prefixes another's does not claim its specifiers.
pub(crate) fn workspace_package_sub_path<'a>(
    specifier: &'a str,
    declared_names: &rustc_hash::FxHashMap<String, i64>,
) -> Option<(i64, &'a str)> {
    if let Some(&id) = declared_names.get(specifier) {
        return Some((id, ""));
    }
    declared_names
        .iter()
        .filter(|(name, _)| {
            specifier
                .strip_prefix(name.as_str())
                .is_some_and(|suffix| suffix.starts_with('/'))
        })
        .max_by_key(|(name, _)| name.len())
        .map(|(name, &id)| (id, &specifier[name.len() + 1..]))
}

/// Match an import spelling to a declared workspace package. Ecosystem
/// adapters own the spelling normalization; the generic resolver only consumes
/// the matched package id.
pub(crate) fn workspace_package_id(
    specifier: &str,
    declared_names: &rustc_hash::FxHashMap<String, i64>,
) -> Option<i64> {
    workspace_package_sub_path(specifier, declared_names).map(|(id, _)| id)
}

/// Construct module resolvers from opaque ecosystem inputs plus neutral
/// workspace ownership evidence. A per-source package owner is supplied by the
/// caller because it is a property of the importing file, not the workspace.
pub(crate) fn language_resolvers(
    inputs: &ResolverInputs,
    source_package_name: Option<&str>,
    workspace_packages: Vec<(String, String)>,
) -> Vec<Box<dyn crate::indexer::module_resolution::ModuleResolver>> {
    crate::indexer::module_resolution::all_resolvers_with_workspace(
        inputs.go_module_path.as_deref(),
        source_package_name,
        workspace_packages,
    )
}

#[cfg(test)]
#[path = "module_specifier_tests.rs"]
mod tests;
