use super::*;

fn package(id: i64, path: &str, declared: &str, kind: &str) -> PackageInfo {
    PackageInfo {
        id: Some(id),
        name: path.rsplit('/').next().unwrap_or(path).to_string(),
        path: path.to_string(),
        kind: Some(kind.to_string()),
        manifest: None,
        declared_name: Some(declared.to_string()),
        is_publishable: true,
    }
}

fn pub_manifests(source_root: Option<&str>) -> HashMap<ManifestKind, ManifestData> {
    let data = ManifestData {
        package_source_root: source_root.map(str::to_string),
        ..Default::default()
    };
    [(ManifestKind::Pubspec, data)].into_iter().collect()
}

/// A mirror copy of a package under a template directory declares the same
/// name; the first package in id order keeps the name slot, and the ecosystem
/// alias for that name routes to the same package rather than the mirror.
#[test]
fn duplicate_declared_name_keeps_the_first_package_for_name_and_alias() {
    let packages = vec![
        package(1, "packages/core_client", "core_client", "dart"),
        package(
            2,
            "templates/pubspecs/packages/core_client",
            "core_client",
            "dart",
        ),
    ];
    let ws = indexes(&packages, &HashMap::new());

    assert_eq!(ws.by_declared_name.get("core_client"), Some(&1));
    assert_eq!(ws.by_declared_name.get("package:core_client"), Some(&1));
}

/// The canonical map answers with the spelling the manifest declared, so a
/// consumer handing the name back to its ecosystem never receives an alias.
#[test]
fn canonical_name_map_holds_only_the_owning_package_and_its_own_spelling() {
    let packages = vec![
        package(1, "packages/core_client", "core_client", "dart"),
        package(
            2,
            "templates/pubspecs/packages/core_client",
            "core_client",
            "dart",
        ),
    ];
    let ws = indexes(&packages, &HashMap::new());

    assert_eq!(
        ws.declared_name.get(&1).map(String::as_str),
        Some("core_client")
    );
    assert_eq!(ws.declared_name.get(&2), None);
    assert!(
        !ws.declared_name
            .values()
            .any(|name| name.starts_with("package:")),
        "an ecosystem alias must never become a package's canonical name"
    );
}

/// A declared source root is stored joined onto the package's own root, so a
/// consumer maps a sub-path without knowing where the package sits.
#[test]
fn declared_source_root_is_joined_onto_the_package_root() {
    let packages = vec![package(1, "packages/core_client", "core_client", "dart")];
    let by_package = [(1, pub_manifests(Some("lib")))].into_iter().collect();

    let ws = indexes(&packages, &by_package);

    assert_eq!(
        ws.source_roots.get(&1).map(String::as_str),
        Some("packages/core_client/lib")
    );
}

#[test]
fn a_package_whose_manifest_declares_no_source_root_has_none() {
    let packages = vec![package(1, "packages/utils", "@org/utils", "npm")];
    let by_package = [(1, pub_manifests(None))].into_iter().collect();

    let ws = indexes(&packages, &by_package);

    assert!(ws.source_roots.is_empty());
}

/// The workspace root package publishes from the root itself, so its source
/// root carries no leading path segment.
#[test]
fn root_package_source_root_has_no_leading_segment() {
    let packages = vec![package(1, "", "demo_workspace", "dart")];
    let by_package = [(1, pub_manifests(Some("lib")))].into_iter().collect();

    let ws = indexes(&packages, &by_package);

    assert_eq!(ws.source_roots.get(&1).map(String::as_str), Some("lib"));
}
