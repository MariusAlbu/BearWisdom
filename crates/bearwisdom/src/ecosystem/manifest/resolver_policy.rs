//! Resolver-facing manifest data assembled by ecosystem-owned contributors.

use std::collections::HashMap;

use super::{ManifestData, ManifestKind};

#[derive(Clone, Default)]
pub(crate) struct ResolverManifestPolicy {
    module_rewrites: Vec<(String, String)>,
    package_aliases: Vec<(String, String)>,
    implicit_namespaces: Vec<String>,
}

impl ResolverManifestPolicy {
    /// Add a source-module spelling rewrite supplied by its owning ecosystem.
    pub(crate) fn add_module_rewrites(
        &mut self,
        rewrites: impl IntoIterator<Item = (String, String)>,
    ) {
        self.module_rewrites.extend(rewrites);
    }

    /// Add consumer-scoped package aliases supplied by their owning ecosystem.
    pub(crate) fn add_package_aliases(
        &mut self,
        aliases: impl IntoIterator<Item = (String, String)>,
    ) {
        self.package_aliases.extend(aliases);
    }

    pub(crate) fn add_implicit_namespaces(&mut self, namespaces: impl IntoIterator<Item = String>) {
        self.implicit_namespaces.extend(namespaces);
    }

    /// Return the canonical module spelling selected by the owning resolver
    /// policy. Longest matching source prefix wins.
    pub(crate) fn resolve_module_alias(&self, specifier: &str) -> Option<String> {
        let (alias, target) = self
            .module_rewrites
            .iter()
            .filter(|(alias, _)| specifier.starts_with(alias))
            .max_by_key(|(alias, _)| alias.len())?;
        Some(format!("{target}{}", &specifier[alias.len()..]))
    }

    /// Return the canonical package name for a consumer-scoped alias.
    pub(crate) fn resolve_package_alias(&self, alias: &str) -> Option<&str> {
        self.package_aliases
            .iter()
            .find(|(source, _)| source == alias)
            .map(|(_, target)| target.as_str())
    }

    pub(crate) fn implicit_namespaces(&self) -> &[String] {
        &self.implicit_namespaces
    }
}

pub(crate) fn from_manifests(
    manifests: &HashMap<ManifestKind, ManifestData>,
) -> ResolverManifestPolicy {
    let mut policy = ResolverManifestPolicy::default();
    crate::ecosystem::npm::resolver_policy::contribute(manifests, &mut policy);
    crate::ecosystem::cargo::resolver_policy::contribute(manifests, &mut policy);
    crate::ecosystem::nuget::resolver_policy::contribute(manifests, &mut policy);
    policy
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecosystem_contributors_supply_only_owned_fields() {
        let manifests = HashMap::from([
            (
                ManifestKind::Npm,
                ManifestData {
                    path_aliases: vec![("@/".into(), "src/".into())],
                    ..Default::default()
                },
            ),
            (
                ManifestKind::Cargo,
                ManifestData {
                    dep_renames: vec![("local".into(), "shared".into())],
                    ..Default::default()
                },
            ),
            (
                ManifestKind::NuGet,
                ManifestData {
                    global_usings: vec!["System".into()],
                    ..Default::default()
                },
            ),
        ]);

        let policy = from_manifests(&manifests);
        assert_eq!(
            policy.resolve_module_alias("@/feature"),
            Some("src/feature".into())
        );
        assert_eq!(policy.resolve_package_alias("local"), Some("shared"));
        assert_eq!(policy.implicit_namespaces(), ["System"]);
    }

    #[test]
    fn non_owner_manifest_cannot_supply_resolver_fields() {
        let manifests = HashMap::from([(
            ManifestKind::PyProject,
            ManifestData {
                path_aliases: vec![("@/".into(), "wrong".into())],
                dep_renames: vec![("wrong".into(), "wrong".into())],
                global_usings: vec!["Wrong".into()],
                ..Default::default()
            },
        )]);
        let policy = from_manifests(&manifests);
        assert!(policy.resolve_module_alias("@/feature").is_none());
        assert!(policy.resolve_package_alias("wrong").is_none());
        assert!(policy.implicit_namespaces().is_empty());
    }
}
