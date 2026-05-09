// =============================================================================
// alire_tests.rs
// =============================================================================

use super::*;

#[test]
fn ecosystem_identity() {
    let e = AlireEcosystem;
    assert_eq!(e.id(), ID);
    assert_eq!(Ecosystem::kind(&e), EcosystemKind::Package);
    assert_eq!(Ecosystem::languages(&e), &["ada"]);
}

#[test]
fn legacy_locator_tag_is_alire() {
    assert_eq!(
        ExternalSourceLocator::ecosystem(&AlireEcosystem),
        "alire"
    );
}

#[test]
fn parse_array_of_tables_form_collects_dep_names() {
    let manifest = r#"
name = "alr"
version = "2.2.0-dev"

[[depends-on]]
aaa = "~0.3.0"

[[depends-on]]
ada_toml = "~0.5"

[[depends-on]]
ansiada = "^1.1"
"#;
    let deps = parse_alire_dependencies_text(manifest);
    assert!(deps.contains(&"aaa".to_string()));
    assert!(deps.contains(&"ada_toml".to_string()));
    assert!(deps.contains(&"ansiada".to_string()));
    assert_eq!(deps.len(), 3);
}

#[test]
fn parse_flat_table_form_collects_dep_names() {
    let manifest = r#"
name = "septum"
version = "0.0.9"

[depends-on]
dir_iterators = "~0.0.5"
progress_indicators = "~0.0.1"
trendy_terminal = "~0.0.5"
"#;
    let deps = parse_alire_dependencies_text(manifest);
    assert!(deps.contains(&"dir_iterators".to_string()));
    assert!(deps.contains(&"progress_indicators".to_string()));
    assert!(deps.contains(&"trendy_terminal".to_string()));
    assert_eq!(deps.len(), 3);
}

#[test]
fn parse_skips_unknown_top_level_sections() {
    // alr 2.2-dev's manifest carries a `[test]` section that older alr
    // rejects. The parser must skip unknown sections without choking.
    let manifest = r#"
name = "alr"

[[depends-on]]
aaa = "~0.3.0"

[test]
runner = "alire"
directory = "testsuite/tests_ada"

[gpr-set-externals]
CLIC_LIBRARY_TYPE = "static"
"#;
    let deps = parse_alire_dependencies_text(manifest);
    assert_eq!(deps, vec!["aaa".to_string()]);
}

#[test]
fn parse_handles_inline_comments_and_quoted_keys() {
    let manifest = r#"
[[depends-on]]
aaa = "~0.3.0"  # Added by alr
"ada_toml" = "~0.5"  # quoted key form
ansiada = "^1.1"
"#;
    let deps = parse_alire_dependencies_text(manifest);
    assert!(deps.contains(&"aaa".to_string()));
    assert!(deps.contains(&"ada_toml".to_string()));
    assert!(deps.contains(&"ansiada".to_string()));
}

#[test]
fn parse_resets_when_leaving_depends_on_section() {
    let manifest = r#"
[[depends-on]]
aaa = "~0.3.0"

[[pins]]
[pins.aaa]
url = "https://github.com/mosteo/aaa"
commit = "deadbeef"
"#;
    let deps = parse_alire_dependencies_text(manifest);
    // `url` and `commit` are inside [[pins]] — must NOT be picked up as deps.
    assert_eq!(deps, vec!["aaa".to_string()]);
}

#[test]
fn parse_returns_empty_when_no_manifest() {
    let tmp = std::env::temp_dir().join("bw-test-alire-no-manifest");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let deps = parse_alire_dependencies(&tmp);
    assert!(deps.is_empty());
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn discover_externals_returns_roots_for_present_deps() {
    // Build a fake Alire cache: <cache>/releases/aaa_0.3.0_abcdef/.
    let tmp = std::env::temp_dir().join("bw-test-alire-discover");
    let _ = std::fs::remove_dir_all(&tmp);
    let cache = tmp.join("cache").join("releases");
    let aaa_dir = cache.join("aaa_0.3.0_abcdef");
    let aaa_old = cache.join("aaa_0.2.0_111111");
    let other_dir = cache.join("ansiada_1.1.0_zzzzzz");
    std::fs::create_dir_all(&aaa_dir).unwrap();
    std::fs::create_dir_all(&aaa_old).unwrap();
    std::fs::create_dir_all(&other_dir).unwrap();
    std::fs::write(aaa_dir.join("dummy.ads"), "package AAA is\nend AAA;\n").unwrap();

    // Project with a manifest declaring aaa.
    let project = tmp.join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("alire.toml"),
        "name = \"demo\"\n[[depends-on]]\naaa = \"~0.3.0\"\n",
    )
    .unwrap();

    let saved = std::env::var_os("BEARWISDOM_ALIRE_CACHE");
    std::env::set_var("BEARWISDOM_ALIRE_CACHE", cache.to_string_lossy().to_string());
    let roots = discover_alire_externals(&project);
    match saved {
        Some(v) => std::env::set_var("BEARWISDOM_ALIRE_CACHE", v),
        None => std::env::remove_var("BEARWISDOM_ALIRE_CACHE"),
    }

    assert_eq!(roots.len(), 1, "expected one root for `aaa`, got: {roots:?}");
    let root = &roots[0];
    assert_eq!(root.module_path, "aaa");
    // Highest version wins: 0.3.0 over 0.2.0.
    assert_eq!(root.version, "0.3.0");
    assert!(root.root.ends_with("aaa_0.3.0_abcdef"));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn walk_collects_ads_and_adb_files() {
    let tmp = std::env::temp_dir().join("bw-test-alire-walk");
    let _ = std::fs::remove_dir_all(&tmp);
    let src = tmp.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("aaa.ads"), "package AAA is\nend AAA;\n").unwrap();
    std::fs::write(src.join("aaa.adb"), "package body AAA is\nend AAA;\n").unwrap();
    // Should be skipped:
    std::fs::create_dir_all(tmp.join("obj")).unwrap();
    std::fs::write(tmp.join("obj").join("ignored.ads"), "package Ignored is\nend Ignored;\n").unwrap();
    std::fs::create_dir_all(tmp.join("alire")).unwrap();
    std::fs::write(tmp.join("alire").join("tracked.ads"), "package Tracked is\nend Tracked;\n").unwrap();

    let dep = ExternalDepRoot {
        module_path: "aaa".to_string(),
        version: "0.3.0".to_string(),
        root: tmp.clone(),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    let files = walk_alire_root(&dep);
    let names: Vec<&str> = files
        .iter()
        .map(|f| f.relative_path.as_str())
        .collect();
    assert!(
        names.iter().any(|n| n.ends_with("aaa.ads")),
        "missing aaa.ads: {names:?}"
    );
    assert!(
        names.iter().any(|n| n.ends_with("aaa.adb")),
        "missing aaa.adb: {names:?}"
    );
    assert!(
        !names.iter().any(|n| n.contains("/obj/")),
        "obj/ should be pruned: {names:?}"
    );
    assert!(
        !names.iter().any(|n| n.contains("/alire/")),
        "alire/ should be pruned: {names:?}"
    );
    for f in &files {
        assert!(f.relative_path.starts_with("ext:alire:"));
        assert_eq!(f.language, "ada");
    }

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn build_symbol_index_indexes_packages_lowercase_and_canonical() {
    let tmp = std::env::temp_dir().join("bw-test-alire-index");
    let _ = std::fs::remove_dir_all(&tmp);
    let src = tmp.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("aaa.ads"), "package AAA is\nend AAA;\n").unwrap();
    std::fs::write(
        src.join("aaa-strings.ads"),
        "package AAA.Strings is\nend AAA.Strings;\n",
    )
    .unwrap();

    let dep = ExternalDepRoot {
        module_path: "aaa".to_string(),
        version: "0.3.0".to_string(),
        root: tmp.clone(),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    let index = build_alire_symbol_index(&[dep]);

    assert!(
        index.locate("aaa", "aaa").is_some(),
        "lowercase AAA should resolve; index size {}",
        index.len()
    );
    assert!(
        index.locate("aaa", "AAA").is_some(),
        "case-preserving AAA should resolve"
    );
    assert!(
        index.locate("aaa", "aaa.strings").is_some(),
        "AAA.Strings (lowercase) should resolve"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn resolve_import_strips_trailing_children() {
    let tmp = std::env::temp_dir().join("bw-test-alire-resolve");
    let _ = std::fs::remove_dir_all(&tmp);
    let src = tmp.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        src.join("aaa-table_map.ads"),
        "package AAA.Table_Map is\nend AAA.Table_Map;\n",
    )
    .unwrap();

    let dep = ExternalDepRoot {
        module_path: "aaa".to_string(),
        version: "0.3.0".to_string(),
        root: tmp.clone(),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    let walked = AlireEcosystem.resolve_symbol(&dep, "AAA.Table_Map.Insert");
    assert_eq!(walked.len(), 1);
    assert!(walked[0].relative_path.contains("aaa-table_map.ads"));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn manifest_reader_kind_is_alire() {
    use crate::ecosystem::manifest::ManifestReader;
    let r = AlireManifest;
    assert_eq!(
        r.kind(),
        crate::ecosystem::manifest::ManifestKind::Alire
    );
}

#[test]
fn _ensure_shared_locator_typed() -> () {
    // Smoke: the shared_locator helper compiles and returns a typed Arc.
    let _arc: std::sync::Arc<dyn ExternalSourceLocator> = shared_locator();
}
