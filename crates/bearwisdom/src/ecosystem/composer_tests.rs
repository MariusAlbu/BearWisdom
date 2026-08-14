use super::*;

#[test]
fn ecosystem_identity() {
    let c = ComposerEcosystem;
    assert_eq!(c.id(), ID);
    assert_eq!(Ecosystem::kind(&c), EcosystemKind::Package);
    assert_eq!(Ecosystem::languages(&c), &["php"]);
}

#[test]
fn legacy_locator_tag_is_php() {
    assert_eq!(ExternalSourceLocator::ecosystem(&ComposerEcosystem), "php");
}

#[test]
fn composer_json_parser_skips_platform_requirements() {
    let content = r#"{"require":{"php":">=8.0","ext-json":"*","laravel/framework":"^11.0"}}"#;
    let deps = parse_composer_json_deps(content);
    assert_eq!(deps, vec!["laravel/framework"]);
}

#[test]
fn php_discovers_composer_deps() {
    let tmp = std::env::temp_dir().join("bw-test-composer-discover");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(
        tmp.join("composer.json"),
        r#"{"require":{"laravel/framework":"^11.0"}}"#,
    )
    .unwrap();
    let vendor = tmp
        .join("vendor")
        .join("laravel")
        .join("framework")
        .join("src");
    std::fs::create_dir_all(&vendor).unwrap();
    std::fs::write(
        vendor.join("Application.php"),
        "<?php class Application {}\n",
    )
    .unwrap();

    let roots = discover_php_externals(&tmp);
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].module_path, "laravel/framework");
    let files = walk_php_root(&roots[0]);
    assert_eq!(files.len(), 1);
    assert!(files[0].relative_path.contains("Application.php"));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[allow(dead_code)]
fn _ensure_shared_locator_typed() -> Arc<dyn ExternalSourceLocator> {
    shared_locator()
}

// ----------------------------------------------------------------
// Header scan + symbol-index offer keys
// ----------------------------------------------------------------

#[test]
fn scan_captures_namespace_and_declarations() {
    let (names, ns) = scan_php_header(
        "<?php declare(strict_types=1);\nnamespace Acme\\Testing;\nuse function sprintf;\nabstract class Harness extends Base implements Runnable {}\ninterface Runnable {}\n",
    );
    assert_eq!(ns.as_deref(), Some("Acme\\Testing"));
    assert!(names.iter().any(|n| n == "Harness"), "got {names:?}");
    assert!(names.iter().any(|n| n == "Runnable"), "got {names:?}");
}

#[test]
fn scan_without_namespace_yields_none() {
    let (names, ns) = scan_php_header("<?php class Plain {}\n");
    assert_eq!(ns, None);
    assert_eq!(names, vec!["Plain".to_string()]);
}

#[test]
fn symbol_index_offers_package_and_namespace_keys() {
    let tmp = std::env::temp_dir().join("bw-test-composer-ns-keys");
    let _ = std::fs::remove_dir_all(&tmp);
    let dep_root = tmp.join("acme").join("harness");
    let src = dep_root.join("src").join("Testing");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        src.join("Harness.php"),
        "<?php\nnamespace Acme\\Testing;\nabstract class Harness {}\n",
    )
    .unwrap();

    let dep = ExternalDepRoot {
        module_path: "acme/harness".to_string(),
        version: "1.0".to_string(),
        root: dep_root.clone(),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    let index = build_php_symbol_index(&[dep]);
    assert!(
        index.locate("acme/harness", "Harness").is_some(),
        "package-name key must offer the declaration"
    );
    assert!(
        index.locate("Acme\\Testing", "Harness").is_some(),
        "namespace key must offer the declaration — `use` statements tag refs with it"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

// ----------------------------------------------------------------
// R3 — user `use` scan + narrowed walk
// ----------------------------------------------------------------

#[test]
fn php_use_extracts_fqn() {
    let mut out = std::collections::HashSet::new();
    extract_php_uses_from_source(
        "<?php\nuse Symfony\\Component\\HttpFoundation\\Request;\nuse App\\Service\\Foo as Bar;\nuse function Some\\helper;\nuse const Foo\\CONST_X;\n",
        &mut out,
    );
    assert!(out.contains("Symfony\\Component\\HttpFoundation\\Request"));
    assert!(out.contains("App\\Service\\Foo"));
    assert!(out.contains("Some\\helper"));
    assert!(out.contains("Foo\\CONST_X"));
}

#[test]
fn php_group_use_explodes() {
    let mut out = std::collections::HashSet::new();
    extract_php_uses_from_source(
        "<?php\nuse Foo\\Bar\\{Baz, Qux as Q, Sub\\Thing};\n",
        &mut out,
    );
    assert!(out.contains("Foo\\Bar\\Baz"));
    assert!(out.contains("Foo\\Bar\\Qux"));
    assert!(out.contains("Foo\\Bar\\Sub\\Thing"));
}

#[test]
fn php_fqn_suffix_uses_last_two_segments() {
    assert_eq!(
        php_fqn_to_path_suffix("Symfony\\Component\\HttpFoundation\\Request"),
        Some("HttpFoundation/Request.php".to_string())
    );
    assert_eq!(
        php_fqn_to_path_suffix("Foo\\Bar"),
        Some("Foo/Bar.php".to_string())
    );
    assert_eq!(php_fqn_to_path_suffix("Foo"), Some("Foo.php".to_string()));
    assert_eq!(php_fqn_to_path_suffix(""), None);
}

#[test]
fn php_narrowed_walk_excludes_unreferenced_packages() {
    let tmp = std::env::temp_dir().join("bw-test-composer-r3-narrow");
    let _ = std::fs::remove_dir_all(&tmp);
    let dep_root = tmp.join("symfony").join("http-foundation");
    let src = dep_root.join("src");
    std::fs::create_dir_all(src.join("HttpFoundation")).unwrap();
    std::fs::create_dir_all(src.join("Unrelated")).unwrap();
    std::fs::write(
        src.join("HttpFoundation/Request.php"),
        "<?php class Request {}\n",
    )
    .unwrap();
    // Same-namespace sibling: included by virtue of Request matching. This
    // mirrors how PHP files reference same-namespace classes without a
    // `use` statement — walking the matched file but not its sibling
    // would leave those references unresolved.
    std::fs::write(
        src.join("HttpFoundation/Response.php"),
        "<?php class Response {}\n",
    )
    .unwrap();
    // Unrelated package (no matching FQN): must not be walked.
    std::fs::write(src.join("Unrelated/Thing.php"), "<?php class Thing {}\n").unwrap();

    let dep = ExternalDepRoot {
        module_path: "symfony/http-foundation".to_string(),
        version: "6.0".to_string(),
        root: dep_root.clone(),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: vec!["Symfony\\Component\\HttpFoundation\\Request".to_string()],
    };
    let files = walk_php_narrowed(&dep);
    let paths: std::collections::HashSet<_> =
        files.iter().map(|f| f.absolute_path.clone()).collect();
    assert!(paths.contains(&src.join("HttpFoundation/Request.php")));
    assert!(
        paths.contains(&src.join("HttpFoundation/Response.php")),
        "same-namespace sibling should be walked: {paths:?}"
    );
    assert!(
        !paths.contains(&src.join("Unrelated/Thing.php")),
        "unrelated package should not be walked: {paths:?}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn php_narrowed_walk_falls_back_when_no_imports() {
    let tmp = std::env::temp_dir().join("bw-test-composer-r3-fallback");
    let _ = std::fs::remove_dir_all(&tmp);
    let dep_root = tmp.join("foo").join("bar");
    let src = dep_root.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("A.php"), "<?php class A {}\n").unwrap();

    let dep = ExternalDepRoot {
        module_path: "foo/bar".to_string(),
        version: "1.0".to_string(),
        root: dep_root.clone(),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    let files = walk_php_narrowed(&dep);
    assert_eq!(files.len(), 1);

    let _ = std::fs::remove_dir_all(&tmp);
}
