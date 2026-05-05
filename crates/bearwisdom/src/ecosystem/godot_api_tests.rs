use super::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn project_gate_rejects_directory_without_project_godot() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("script.gd"), "extends Node\n").unwrap();
    assert!(!project_has_godot_manifest(tmp.path()));
}

#[test]
fn project_gate_accepts_root_project_godot() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("project.godot"), "config_version=5\n").unwrap();
    assert!(project_has_godot_manifest(tmp.path()));
}

#[test]
fn project_gate_finds_nested_project_godot() {
    let tmp = TempDir::new().unwrap();
    let nested = tmp.path().join("apps/game");
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("project.godot"), "config_version=5\n").unwrap();
    assert!(project_has_godot_manifest(tmp.path()));
}

#[test]
fn project_gate_skips_well_known_dirs() {
    let tmp = TempDir::new().unwrap();
    let cached = tmp.path().join("node_modules/godot-game");
    fs::create_dir_all(&cached).unwrap();
    fs::write(cached.join("project.godot"), "config_version=5\n").unwrap();
    assert!(
        !project_has_godot_manifest(tmp.path()),
        "node_modules-vendored project.godot must not activate the ecosystem"
    );
}

#[test]
fn locate_roots_empty_when_no_project_godot() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("stray.gd"), "extends Node\n").unwrap();

    let e = GodotApiEcosystem;
    let ctx = LocateContext {
        project_root: tmp.path(),
        manifests: &Default::default(),
        active_ecosystems: &[],
    };
    let roots = Ecosystem::locate_roots(&e, &ctx);
    assert!(roots.is_empty());
}

#[test]
fn parse_metadata_only_returns_none_without_project_godot() {
    let tmp = TempDir::new().unwrap();
    let e = GodotApiEcosystem;
    assert!(ExternalSourceLocator::parse_metadata_only(&e, tmp.path()).is_none());
}

#[test]
fn ecosystem_identity_unchanged() {
    let e = GodotApiEcosystem;
    assert_eq!(e.id(), ID);
    assert_eq!(Ecosystem::kind(&e), EcosystemKind::Stdlib);
    assert_eq!(Ecosystem::languages(&e), &["gdscript"]);
}
