use super::*;
use crate::ecosystem::manifest::module_config::ModulePackage;

#[test]
fn union_keeps_package_configuration_addresses_distinct() {
    let packages: Vec<_> = ["one", "two"]
        .into_iter()
        .map(|name| PackageManifest {
            name: name.into(),
            path: name.into(),
            kind: ManifestKind::Cargo,
            manifest_path: format!("{name}/Cargo.toml").into(),
            data: ManifestData {
                module_packages: vec![ModulePackage {
                    root: name.into(),
                    name: name.into(),
                    fingerprint: name.into(),
                    targets: Vec::new(),
                    dependencies: Vec::new(),
                }],
                ..Default::default()
            },
        })
        .collect();
    let union = union_manifests(&packages);
    assert_eq!(
        union[&ManifestKind::Cargo]
            .module_packages
            .iter()
            .map(|p| p.root.as_str())
            .collect::<Vec<_>>(),
        ["one", "two"]
    );
}
