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

const ADAPTERS: &[Adapter] = &[Adapter {
    entry_aliases: super::pub_pkg::module_specifier::entry_aliases,
    relative_entry_key: super::pub_pkg::module_specifier::relative_entry_key,
}];

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

/// Match an import spelling to a declared workspace package. Ecosystem
/// adapters own the spelling normalization; the generic resolver only consumes
/// the matched package id.
pub(crate) fn workspace_package_id(
    specifier: &str,
    declared_names: &rustc_hash::FxHashMap<String, i64>,
) -> Option<i64> {
    declared_names.get(specifier).copied().or_else(|| {
        declared_names
            .iter()
            .filter(|(name, _)| {
                specifier
                    .strip_prefix(name.as_str())
                    .is_some_and(|suffix| suffix.starts_with('/'))
            })
            .max_by_key(|(name, _)| name.len())
            .map(|(_, &id)| id)
    })
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
mod tests {
    use super::package_entry_key;

    #[test]
    fn delegates_scoped_and_unscoped_external_package_keys() {
        assert_eq!(
            package_entry_key("ext:ts:@scope/pkg/dist/index.d.ts").as_deref(),
            Some("@scope/pkg")
        );
        assert_eq!(
            package_entry_key("ext:ruby:devise/lib/devise.rb").as_deref(),
            Some("devise")
        );
        assert_eq!(
            package_entry_key("ext:unknown:pkg/file"),
            None,
            "an unowned virtual-path scheme must not receive package semantics"
        );
    }
}
