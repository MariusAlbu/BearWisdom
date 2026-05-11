use super::*;
use crate::types::PackageInfo;

#[test]
fn declared_name_round_trips_through_db() {
    let db = Database::open_in_memory().unwrap();
    let packages = vec![
        PackageInfo {
            id: None,
            name: "web".into(),
            path: "web".into(),
            kind: Some("npm".into()),
            manifest: Some("web/package.json".into()),
            declared_name: Some("@myorg/web".into()),
        },
        PackageInfo {
            id: None,
            name: "shared".into(),
            path: "shared".into(),
            kind: Some("npm".into()),
            manifest: Some("shared/package.json".into()),
            declared_name: Some("@myorg/shared".into()),
        },
    ];

    let written = write_packages(&db, &packages).unwrap();
    assert_eq!(written.len(), 2);

    let loaded = load_packages_from_db(&db).unwrap();
    let web = loaded.iter().find(|p| p.name == "web").expect("web");
    let shared = loaded.iter().find(|p| p.name == "shared").expect("shared");
    assert_eq!(web.declared_name.as_deref(), Some("@myorg/web"));
    assert_eq!(shared.declared_name.as_deref(), Some("@myorg/shared"));
}

#[test]
fn declared_name_nullable_when_absent() {
    let db = Database::open_in_memory().unwrap();
    let packages = vec![PackageInfo {
        id: None,
        name: "legacy".into(),
        path: "legacy".into(),
        kind: None,
        manifest: None,
        declared_name: None,
    }];
    write_packages(&db, &packages).unwrap();
    let loaded = load_packages_from_db(&db).unwrap();
    assert_eq!(loaded.len(), 1);
    assert!(loaded[0].declared_name.is_none());
}

fn pf(path: &str) -> crate::types::ParsedFile {
    crate::types::ParsedFile {
        path: path.to_string(),
        language: "rust".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols: Vec::new(),
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: crate::types::FlowMeta::default(),
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
    }
}

/// A root package with `path = ""` must claim every file in the project
/// when no deeper-prefix package matches. The empty-path entry sorts last
/// (length desc), so it only wins when no proper-prefix package matched.
#[test]
fn assign_package_ids_root_package_claims_all_files() {
    let packages = vec![PackageInfo {
        id: Some(7),
        name: "root".into(),
        path: String::new(),
        kind: Some("cargo".into()),
        manifest: Some("Cargo.toml".into()),
        declared_name: Some("root".into()),
    }];

    let mut parsed = vec![pf("src/lib.rs"), pf("examples/demo.rs"), pf("Cargo.toml")];

    assign_package_ids(&mut parsed, &packages);
    for p in &parsed {
        assert_eq!(p.package_id, Some(7), "root package should claim {}", p.path);
    }
}

/// Deeper-prefix packages must beat the root package even though the root
/// entry's `starts_with("")` always returns true. Sort order ensures the
/// real prefix is tried first; the empty-path entry is the fallback.
#[test]
fn assign_package_ids_deeper_prefix_beats_root() {
    let packages = vec![
        PackageInfo {
            id: Some(1),
            name: "root".into(),
            path: String::new(),
            kind: Some("npm".into()),
            manifest: Some("package.json".into()),
            declared_name: Some("root".into()),
        },
        PackageInfo {
            id: Some(2),
            name: "web".into(),
            path: "apps/web".into(),
            kind: Some("npm".into()),
            manifest: Some("apps/web/package.json".into()),
            declared_name: Some("@org/web".into()),
        },
    ];

    let mut parsed = vec![pf("apps/web/src/index.ts"), pf("tools/lint.ts")];

    assign_package_ids(&mut parsed, &packages);
    assert_eq!(parsed[0].package_id, Some(2), "web file picks the deeper package");
    assert_eq!(parsed[1].package_id, Some(1), "unrelated file falls through to root");
}
