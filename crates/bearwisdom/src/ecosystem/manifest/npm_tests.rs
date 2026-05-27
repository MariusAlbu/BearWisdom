use super::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[test]
fn parses_name_and_registry_deps() {
    let json = r#"{"name": "@myorg/web", "dependencies": {"react": "^18.0.0", "axios": "1.2.3"}}"#;
    let (name, deps) = parse_package_json(json);
    assert_eq!(name.as_deref(), Some("@myorg/web"));
    assert!(deps.contains(&"react".to_string()));
    assert!(deps.contains(&"axios".to_string()));
}

#[test]
fn workspace_protocol_deps_excluded() {
    let json = r#"{
        "name": "@myorg/web",
        "dependencies": {
            "react": "^18.0.0",
            "@myorg/utils": "workspace:*",
            "@myorg/ui": "workspace:^",
            "@myorg/local": "file:../local-pkg"
        }
    }"#;
    let (_, deps) = parse_package_json(json);
    assert!(deps.contains(&"react".to_string()), "registry dep kept");
    assert!(!deps.contains(&"@myorg/utils".to_string()), "workspace:* excluded");
    assert!(!deps.contains(&"@myorg/ui".to_string()), "workspace:^ excluded");
    assert!(!deps.contains(&"@myorg/local".to_string()), "file: excluded");
}

#[test]
fn link_and_portal_protocols_excluded() {
    let json = r#"{
        "name": "app",
        "dependencies": {
            "pinned": "link:../pinned",
            "tunneled": "portal:../tunneled"
        }
    }"#;
    let (_, deps) = parse_package_json(json);
    assert!(deps.is_empty(), "link: and portal: both excluded");
}

#[test]
fn mixed_workspace_and_registry_both_work() {
    let json = r#"{
        "name": "web",
        "dependencies": {"react": "^18"},
        "devDependencies": {
            "typescript": "^5",
            "@internal/test-utils": "workspace:*"
        }
    }"#;
    let (_, deps) = parse_package_json(json);
    assert!(deps.contains(&"react".to_string()));
    assert!(deps.contains(&"typescript".to_string()));
    assert!(!deps.contains(&"@internal/test-utils".to_string()));
}

#[test]
fn workspace_protocol_helper_recognizes_variants() {
    assert!(is_workspace_protocol("workspace:*"));
    assert!(is_workspace_protocol("workspace:^"));
    assert!(is_workspace_protocol("workspace:~"));
    assert!(is_workspace_protocol("workspace:1.2.3"));
    assert!(is_workspace_protocol("file:../foo"));
    assert!(is_workspace_protocol("link:../foo"));
    assert!(is_workspace_protocol("portal:../foo"));

    assert!(!is_workspace_protocol("^1.0.0"));
    assert!(!is_workspace_protocol("1.2.3"));
    assert!(!is_workspace_protocol("git+https://github.com/x/y"));
    assert!(!is_workspace_protocol("npm:foo@1.0.0"));
}

// --- BIND-2b: tsconfig `extends` chain following ---

/// Drive `collect_tsconfig_paths` against an in-memory file map so the
/// extends-resolution logic is exercised without touching the filesystem.
/// Paths are built with the same `join` calls the production code uses, so
/// candidate keys match regardless of the platform path separator.
fn paths_via(start: &Path, files: Vec<(PathBuf, &str)>) -> Vec<(String, String)> {
    let map: HashMap<PathBuf, String> =
        files.into_iter().map(|(p, c)| (p, c.to_string())).collect();
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    collect_tsconfig_paths(start, &|p| map.get(p).cloned(), &mut out, &mut seen, 0);
    out
}

#[test]
fn extends_relative_inherits_parent_paths() {
    let root = Path::new("proj");
    let child = root.join("tsconfig.json");
    let base = root.join("tsconfig.base.json");
    let out = paths_via(
        &child,
        vec![
            (child.clone(), r#"{"extends":"./tsconfig.base.json"}"#),
            (base, r#"{"compilerOptions":{"paths":{"@/*":["src/*"]}}}"#),
        ],
    );
    assert_eq!(out, vec![("@/".to_string(), "src/".to_string())]);
}

#[test]
fn child_paths_shadow_parent_on_conflict() {
    let root = Path::new("proj");
    let child = root.join("tsconfig.json");
    let base = root.join("tsconfig.base.json");
    let out = paths_via(
        &child,
        vec![
            (
                child.clone(),
                r#"{"extends":"./tsconfig.base.json","compilerOptions":{"paths":{"@/*":["app/*"]}}}"#,
            ),
            (
                base,
                r#"{"compilerOptions":{"paths":{"@/*":["src/*"],"~/*":["lib/*"]}}}"#,
            ),
        ],
    );
    assert!(out.contains(&("@/".to_string(), "app/".to_string())), "child @/ wins");
    assert!(out.contains(&("~/".to_string(), "lib/".to_string())), "parent ~/ inherited");
    assert!(!out.contains(&("@/".to_string(), "src/".to_string())), "parent @/ shadowed");
}

#[test]
fn extends_package_preset_in_node_modules() {
    let root = Path::new("proj");
    let child = root.join("tsconfig.json");
    let preset = root
        .join("node_modules")
        .join("@tsconfig/base/tsconfig.json");
    let out = paths_via(
        &child,
        vec![
            (child.clone(), r#"{"extends":"@tsconfig/base/tsconfig.json"}"#),
            (preset, r#"{"compilerOptions":{"paths":{"@/*":["src/*"]}}}"#),
        ],
    );
    assert_eq!(out, vec![("@/".to_string(), "src/".to_string())]);
}

#[test]
fn extends_cycle_terminates() {
    let root = Path::new("proj");
    let a = root.join("a.json");
    let b = root.join("b.json");
    let out = paths_via(
        &a,
        vec![
            (
                a.clone(),
                r#"{"extends":"./b.json","compilerOptions":{"paths":{"@/*":["src/*"]}}}"#,
            ),
            (b, r#"{"extends":"./a.json"}"#),
        ],
    );
    assert_eq!(out, vec![("@/".to_string(), "src/".to_string())]);
}

#[test]
fn no_extends_matches_plain_parse() {
    let root = Path::new("proj");
    let child = root.join("tsconfig.json");
    let out = paths_via(
        &child,
        vec![(
            child.clone(),
            r#"{"compilerOptions":{"paths":{"@/*":["src/*"]}}}"#,
        )],
    );
    assert_eq!(out, vec![("@/".to_string(), "src/".to_string())]);
}
