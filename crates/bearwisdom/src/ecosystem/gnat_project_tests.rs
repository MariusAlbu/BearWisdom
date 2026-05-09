// =============================================================================
// gnat_project_tests.rs
// =============================================================================

use super::*;

#[test]
fn ecosystem_identity() {
    let e = GnatProjectEcosystem;
    assert_eq!(e.id(), ID);
    assert_eq!(Ecosystem::kind(&e), EcosystemKind::Package);
    assert_eq!(Ecosystem::languages(&e), &["ada"]);
}

#[test]
fn legacy_locator_tag_is_gnat_project() {
    assert_eq!(
        ExternalSourceLocator::ecosystem(&GnatProjectEcosystem),
        "gnat-project"
    );
}

#[test]
fn parse_gpr_collects_with_clauses() {
    let gpr = r#"
with "../../boards/HiFive1/hifive1_zfp.gpr";
with "shared/util.gpr";
project Demo is
   for Source_Dirs use ("src/");
end Demo;
"#;
    let parsed = parse_gpr_text(gpr);
    assert_eq!(
        parsed.with_paths,
        vec![
            "../../boards/HiFive1/hifive1_zfp.gpr".to_string(),
            "shared/util.gpr".to_string(),
        ]
    );
}

#[test]
fn parse_gpr_collects_simple_source_dirs() {
    let gpr = r#"
project Demo is
   for Source_Dirs use ("src/", "extra/", "components/src/**");
end Demo;
"#;
    let parsed = parse_gpr_text(gpr);
    assert_eq!(
        parsed.source_dirs,
        vec![
            "src/".to_string(),
            "extra/".to_string(),
            "components/src/**".to_string(),
        ]
    );
}

#[test]
fn parse_gpr_resolves_variable_concatenation() {
    // Real-world Alire-generated GPR shape: `Src_Dirs_Root := "../..";` then
    // `Src_Dirs_Root & "/hal/src/"` inside Source_Dirs.
    let gpr = r#"
project Demo is
   Src_Dirs_Root := "../..";
   for Source_Dirs use (
       Src_Dirs_Root & "/hal/src/",
       Src_Dirs_Root & "/boards/native/src/",
       "config_src/"
   );
end Demo;
"#;
    let parsed = parse_gpr_text(gpr);
    assert_eq!(
        parsed.source_dirs,
        vec![
            "../../hal/src/".to_string(),
            "../../boards/native/src/".to_string(),
            "config_src/".to_string(),
        ]
    );
    assert_eq!(
        parsed.variables.get("Src_Dirs_Root"),
        Some(&"../..".to_string())
    );
}

#[test]
fn parse_gpr_skips_complex_expressions_silently() {
    // `Compiler'Default_Switches ("Ada")` and case blocks aren't a string
    // expression — the entry is dropped, others survive.
    let gpr = r#"
project Demo is
   for Source_Dirs use ("src/", Unknown_Var & "/foo", "config/");
end Demo;
"#;
    let parsed = parse_gpr_text(gpr);
    assert_eq!(
        parsed.source_dirs,
        vec!["src/".to_string(), "config/".to_string()]
    );
}

#[test]
fn parse_gpr_strips_comments() {
    let gpr = r#"
-- Heading comment
with "lib.gpr"; -- trailing comment
project Demo is
   for Source_Dirs use (
       "src/", -- inline comment
       "extra/" -- before close paren
   );
end Demo;
"#;
    let parsed = parse_gpr_text(gpr);
    assert_eq!(parsed.with_paths, vec!["lib.gpr".to_string()]);
    assert_eq!(
        parsed.source_dirs,
        vec!["src/".to_string(), "extra/".to_string()]
    );
}

#[test]
fn discover_yields_external_with_when_path_escapes_root() {
    // Simulate two sibling GPR-managed projects: project-a (the one we
    // index) `with`-s project-b which lives outside project-a's tree.
    let tmp = std::env::temp_dir().join("bw-test-gpr-discover");
    let _ = std::fs::remove_dir_all(&tmp);
    let project_a = tmp.join("project_a");
    let project_b = tmp.join("project_b");
    std::fs::create_dir_all(&project_a).unwrap();
    std::fs::create_dir_all(&project_b).unwrap();

    std::fs::write(
        project_a.join("a.gpr"),
        "with \"../project_b/b.gpr\";\nproject A is\n   for Source_Dirs use (\"src/\");\nend A;\n",
    )
    .unwrap();
    std::fs::write(
        project_b.join("b.gpr"),
        "project B is\n   for Source_Dirs use (\"src/\");\nend B;\n",
    )
    .unwrap();
    std::fs::create_dir_all(project_a.join("src")).unwrap();
    std::fs::create_dir_all(project_b.join("src")).unwrap();
    std::fs::write(project_b.join("src").join("b.ads"), "package B is\nend B;\n").unwrap();

    let roots = discover_gnat_project_externals(&project_a);
    assert_eq!(
        roots.len(),
        1,
        "expected one external root for project_b, got {roots:?}"
    );
    let root = &roots[0];
    assert!(
        root.root.ends_with("project_b"),
        "root path mismatch: {:?}",
        root.root
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn discover_skips_internal_with_paths() {
    // When all `with` clauses point inside project_root, no externals
    // are produced — the project walker already covers those files.
    let tmp = std::env::temp_dir().join("bw-test-gpr-internal-only");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let sub = tmp.join("sub");
    std::fs::create_dir_all(&sub).unwrap();

    std::fs::write(
        tmp.join("top.gpr"),
        "with \"sub/inner.gpr\";\nproject Top is\n   for Source_Dirs use (\"src/\");\nend Top;\n",
    )
    .unwrap();
    std::fs::write(
        sub.join("inner.gpr"),
        "project Inner is\n   for Source_Dirs use (\"src/\");\nend Inner;\n",
    )
    .unwrap();

    let roots = discover_gnat_project_externals(&tmp);
    assert!(
        roots.is_empty(),
        "expected no externals for internal-only project, got {roots:?}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn walk_collects_ads_and_adb_files() {
    let tmp = std::env::temp_dir().join("bw-test-gpr-walk");
    let _ = std::fs::remove_dir_all(&tmp);
    let src = tmp.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("foo.ads"), "package Foo is\nend Foo;\n").unwrap();
    std::fs::write(src.join("foo.adb"), "package body Foo is\nend Foo;\n").unwrap();
    // Pruned:
    std::fs::create_dir_all(tmp.join("obj")).unwrap();
    std::fs::write(tmp.join("obj").join("noise.ads"), "package Noise is\nend Noise;\n").unwrap();

    let dep = ExternalDepRoot {
        module_path: "demo".to_string(),
        version: "".to_string(),
        root: tmp.clone(),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    let files = walk_gpr_root(&dep);
    let names: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();
    assert!(names.iter().any(|n| n.ends_with("foo.ads")));
    assert!(names.iter().any(|n| n.ends_with("foo.adb")));
    assert!(!names.iter().any(|n| n.contains("/obj/")));
    for f in &files {
        assert!(f.relative_path.starts_with("ext:gnat-project:"));
        assert_eq!(f.language, "ada");
    }

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn build_symbol_index_indexes_packages() {
    let tmp = std::env::temp_dir().join("bw-test-gpr-index");
    let _ = std::fs::remove_dir_all(&tmp);
    let src = tmp.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("foo.ads"), "package Foo is\nend Foo;\n").unwrap();
    std::fs::write(
        src.join("foo-bar.ads"),
        "package Foo.Bar is\nend Foo.Bar;\n",
    )
    .unwrap();

    let dep = ExternalDepRoot {
        module_path: "demo".to_string(),
        version: "".to_string(),
        root: tmp.clone(),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    let index = build_gnat_project_symbol_index(&[dep]);

    assert!(index.locate("demo", "foo").is_some());
    assert!(index.locate("demo", "Foo").is_some());
    assert!(index.locate("demo", "foo.bar").is_some());

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn split_top_level_ignores_quoted_commas_and_parens() {
    let v = split_top_level("\"a, b\", Var & \"/x\", (1, 2)", ',');
    assert_eq!(v.len(), 3);
    assert_eq!(v[0].trim(), "\"a, b\"");
    assert_eq!(v[1].trim(), "Var & \"/x\"");
    assert_eq!(v[2].trim(), "(1, 2)");
}

#[test]
fn _ensure_shared_locator_typed() -> () {
    let _arc: std::sync::Arc<dyn ExternalSourceLocator> = shared_locator();
}
