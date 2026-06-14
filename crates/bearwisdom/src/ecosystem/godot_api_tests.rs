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

// ---------------------------------------------------------------------------
// Demand-driven interface
// ---------------------------------------------------------------------------

/// Minimal extension_api.json with one class, one builtin, one singleton,
/// one utility function, one global enum, and one global constant.
fn write_minimal_api_json(path: &std::path::Path) {
    let json = serde_json::json!({
        "classes": [
            {
                "name": "Node",
                "inherits": "Object",
                "methods": [
                    { "name": "add_child", "return_value": { "type": "void" }, "arguments": [] }
                ],
                "properties": [
                    { "name": "name", "type": "StringName" }
                ],
                "signals": [
                    { "name": "ready" }
                ],
                "constants": [
                    { "name": "NOTIFICATION_READY" }
                ],
                "enums": [
                    {
                        "name": "ProcessMode",
                        "values": [
                            { "name": "PROCESS_MODE_PAUSABLE" }
                        ]
                    }
                ]
            }
        ],
        "builtin_classes": [
            {
                "name": "Vector2",
                "methods": [
                    { "name": "normalized" }
                ]
            }
        ],
        "singletons": [
            { "name": "Input", "type": "Input" }
        ],
        "utility_functions": [
            { "name": "print", "return_type": "void", "arguments": [] }
        ],
        "global_enums": [
            {
                "name": "Side",
                "values": [
                    { "name": "SIDE_LEFT" }
                ]
            }
        ],
        "global_constants": [
            { "name": "SPKEY" }
        ]
    });
    fs::write(path, json.to_string()).unwrap();
}

fn make_dep(json_path: &std::path::Path) -> ExternalDepRoot {
    ExternalDepRoot {
        module_path: "godot-api".to_string(),
        version: String::new(),
        root: json_path.to_path_buf(),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }
}

#[test]
fn ecosystem_declares_demand_driven() {
    assert!(GodotApiEcosystem.uses_demand_driven_parse());
}

#[test]
fn walk_root_is_empty() {
    let tmp = TempDir::new().unwrap();
    let json = tmp.path().join("extension_api.json");
    write_minimal_api_json(&json);
    let dep = make_dep(&json);
    assert!(Ecosystem::walk_root(&GodotApiEcosystem, &dep).is_empty());
}

#[test]
fn symbol_index_registers_class_names() {
    let tmp = TempDir::new().unwrap();
    let json = tmp.path().join("extension_api.json");
    write_minimal_api_json(&json);

    let mut idx = SymbolLocationIndex::new();
    _test_index_extension_api_json(&json, "godot-api", &mut idx);

    assert!(!idx.is_empty());
    assert!(idx.locate("godot-api", "Node").is_some(), "class name must be indexed");
    assert!(idx.locate("godot-api", "Vector2").is_some(), "builtin class must be indexed");
}

#[test]
fn symbol_index_registers_qualified_member_names() {
    let tmp = TempDir::new().unwrap();
    let json = tmp.path().join("extension_api.json");
    write_minimal_api_json(&json);

    let mut idx = SymbolLocationIndex::new();
    _test_index_extension_api_json(&json, "godot-api", &mut idx);

    assert!(idx.locate("godot-api", "Node.add_child").is_some(), "method must be registered qualified");
    assert!(idx.locate("godot-api", "Node.name").is_some(), "property must be registered qualified");
    assert!(idx.locate("godot-api", "Node.ready").is_some(), "signal must be registered qualified");
    assert!(idx.locate("godot-api", "Node.NOTIFICATION_READY").is_some(), "constant must be registered qualified");
    assert!(idx.locate("godot-api", "Node.ProcessMode").is_some(), "enum must be registered qualified");
    // Bare member names also registered for chain-walker misses.
    assert!(idx.locate("godot-api", "add_child").is_some(), "bare method name must be findable");
}

#[test]
fn symbol_index_registers_globals() {
    let tmp = TempDir::new().unwrap();
    let json = tmp.path().join("extension_api.json");
    write_minimal_api_json(&json);

    let mut idx = SymbolLocationIndex::new();
    _test_index_extension_api_json(&json, "godot-api", &mut idx);

    assert!(idx.locate("godot-api", "Input").is_some(), "singleton must be indexed");
    assert!(idx.locate("godot-api", "print").is_some(), "utility function must be indexed");
    assert!(idx.locate("godot-api", "Side").is_some(), "global enum must be indexed");
    assert!(idx.locate("godot-api", "SIDE_LEFT").is_some(), "global enum value must be indexed");
    assert!(idx.locate("godot-api", "SPKEY").is_some(), "global constant must be indexed");
}

#[test]
fn symbol_index_empty_for_missing_json() {
    let mut idx = SymbolLocationIndex::new();
    _test_index_extension_api_json(std::path::Path::new("/nonexistent/extension_api.json"), "godot-api", &mut idx);
    assert!(idx.is_empty());
}

#[test]
fn build_symbol_index_via_trait() {
    let tmp = TempDir::new().unwrap();
    let json = tmp.path().join("extension_api.json");
    write_minimal_api_json(&json);
    let dep = make_dep(&json);
    let idx = Ecosystem::build_symbol_index(&GodotApiEcosystem, &[dep]);
    assert!(!idx.is_empty());
    assert!(idx.locate("godot-api", "Node").is_some());
}
