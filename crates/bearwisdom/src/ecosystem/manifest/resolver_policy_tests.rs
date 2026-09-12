// =============================================================================
// resolver_policy_tests — module alias rewrites
// =============================================================================

use super::*;
use std::collections::HashMap;

fn policy() -> ResolverManifestPolicy {
    let mut policy = ResolverManifestPolicy::default();
    policy.add_module_rewrites([
        ("@/".to_string(), "src/".to_string()),
        ("@/ui/".to_string(), "packages/ui/src/".to_string()),
    ]);
    policy.add_exact_module_rewrites([(
        "next-test-utils".to_string(),
        "./test/lib/next-test-utils".to_string(),
    )]);
    policy
}

#[test]
fn an_exact_alias_rewrites_only_its_own_specifier() {
    let policy = policy();
    assert_eq!(
        policy.resolve_module_alias("next-test-utils").as_deref(),
        Some("./test/lib/next-test-utils")
    );
    assert_eq!(
        policy.resolve_module_alias("next-test-utils-extra"),
        None,
        "an exact alias is not a prefix"
    );
    assert_eq!(policy.resolve_module_alias("next-test-utils/sub"), None);
}

#[test]
fn the_longest_prefix_alias_wins_and_carries_the_remainder() {
    let policy = policy();
    assert_eq!(
        policy.resolve_module_alias("@/ui/button").as_deref(),
        Some("packages/ui/src/button")
    );
    assert_eq!(
        policy.resolve_module_alias("@/lib/x").as_deref(),
        Some("src/lib/x")
    );
    assert_eq!(policy.resolve_module_alias("react"), None);
}

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
