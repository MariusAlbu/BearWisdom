use super::*;

fn put(root: &Path, path: &str, source: &str) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, source).unwrap();
}

#[test]
fn custom_library_and_explicit_target_paths_override_conventions() {
    let dir = tempfile::tempdir().unwrap();
    for path in [
        "src/lib.rs",
        "src/main.rs",
        "benches/probe.rs",
        "src/bin/tool.rs",
    ] {
        put(dir.path(), path, "");
    }
    let config = configuration("[package]\nname='pkg-name'\nautobins=false\n[lib]\nname='api'\npath='custom/root.rs'\n[[bin]]\nname='tool'\npath='tools/start.rs'", dir.path(), dir.path()).unwrap();
    assert_eq!(
        config
            .targets
            .iter()
            .map(|t| t.path.as_str())
            .collect::<Vec<_>>(),
        ["custom/root.rs", "tools/start.rs", "benches/probe.rs"]
    );
    assert_eq!(config.targets[0].name, "api");
}

#[test]
fn inheritance_preserves_workspace_relative_dependency_locations_and_renames() {
    let dir = tempfile::tempdir().unwrap();
    put(
        dir.path(),
        "Cargo.toml",
        "[workspace.dependencies]\naliased={ package='actual-pkg', path='libs/actual' }",
    );
    put(dir.path(), "consumer/Cargo.toml", "[package]\nname='consumer'\n[dependencies]\naliased.workspace=true\n[dev-dependencies.helper]\npackage='helper-pkg'\npath='../helper'\n[target.'cfg(windows)'.dependencies]\nplatform={path='../platform'}");
    let entry = entry(dir.path().join("consumer/Cargo.toml"), dir.path()).unwrap();
    let config = &entry.data.module_packages[0];
    assert_eq!(config.root, "consumer");
    assert_eq!(config.dependencies[0].root.as_deref(), Some("libs/actual"));
    assert!(config.dependencies[0].renamed);
    assert_eq!(config.dependencies[1].kind, TargetKind::Development);
    assert!(config.dependencies[2].conditional);
}

#[test]
fn invalid_manifests_and_virtual_workspaces_do_not_fabricate_crate_roots() {
    let dir = tempfile::tempdir().unwrap();
    assert!(configuration("[package", dir.path(), dir.path()).is_none());
    assert!(configuration("[workspace]\nmembers=[]", dir.path(), dir.path()).is_none());
    let empty = configuration("[package]\nname='empty'", dir.path(), dir.path()).unwrap();
    assert!(empty.targets.is_empty());
    put(dir.path(), "src/lib.rs", "");
    assert!(configuration(
        "[package]\nname='empty'\nautolib=false",
        dir.path(),
        dir.path()
    )
    .unwrap()
    .targets
    .is_empty());
}

#[test]
fn target_discovery_and_inherited_manifest_edits_change_fingerprints() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = "[package]\nname='demo'";
    let first = configuration(manifest, dir.path(), dir.path()).unwrap();
    put(dir.path(), "tests/acceptance/main.rs", "");
    let second = configuration(manifest, dir.path(), dir.path()).unwrap();
    assert_ne!(first.fingerprint, second.fingerprint);
    assert_eq!(second.targets[0].path, "tests/acceptance/main.rs");
    put(
        dir.path(),
        "Cargo.toml",
        "[workspace.dependencies]\ndep='1'",
    );
    let third = configuration(manifest, dir.path(), dir.path()).unwrap();
    assert_ne!(second.fingerprint, third.fingerprint);
}
