// =============================================================================
// ecosystem/prolog_runtime_tests.rs — sibling tests for prolog_runtime.rs
// =============================================================================

use super::*;
use std::fs;

/// Builds a synthetic SWI-Prolog source layout in a tempdir, then asserts
/// `looks_like_swipl_source` accepts it. The probe checks for `library/`
/// containing a `lists.pl` file — anything matching that shape passes.
#[test]
fn looks_like_swipl_source_accepts_synthetic_layout() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    let library = root.join("library");
    fs::create_dir_all(&library).unwrap();
    fs::write(library.join("lists.pl"), b"% stub").unwrap();

    assert!(looks_like_swipl_source(root));
}

#[test]
fn looks_like_swipl_source_rejects_arbitrary_directory() {
    let dir = tempfile::tempdir().expect("tempdir");
    assert!(!looks_like_swipl_source(dir.path()));
}

#[test]
fn looks_like_swipl_source_rejects_library_without_lists_pl() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::create_dir_all(dir.path().join("library")).unwrap();
    assert!(!looks_like_swipl_source(dir.path()));
}

/// Discovery returns both `library/` and `boot/` roots when the layout
/// exposes them; only `library/` when `boot/` is missing.
#[test]
fn discovery_returns_library_and_boot_when_present() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    let library = root.join("library");
    let boot = root.join("boot");
    fs::create_dir_all(&library).unwrap();
    fs::create_dir_all(&boot).unwrap();
    fs::write(library.join("lists.pl"), b"% stub").unwrap();

    // Set the env var to point at the synthetic layout — this isolates
    // the test from whatever's installed on the host machine.
    std::env::set_var("BEARWISDOM_SWIPL_SOURCE", root);
    let roots = discover_swipl_roots();
    std::env::remove_var("BEARWISDOM_SWIPL_SOURCE");

    assert_eq!(roots.len(), 2, "expected library + boot, got {roots:?}");
    let module_paths: Vec<&str> = roots.iter().map(|r| r.module_path.as_str()).collect();
    assert!(module_paths.contains(&"swipl/library"));
    assert!(module_paths.contains(&"swipl/boot"));
}

#[test]
fn discovery_returns_only_library_when_boot_absent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    let library = root.join("library");
    fs::create_dir_all(&library).unwrap();
    fs::write(library.join("lists.pl"), b"% stub").unwrap();

    std::env::set_var("BEARWISDOM_SWIPL_SOURCE", root);
    let roots = discover_swipl_roots();
    std::env::remove_var("BEARWISDOM_SWIPL_SOURCE");

    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].module_path, "swipl/library");
}

#[test]
fn parse_pllibdir_handles_sh_quoted_form() {
    let text = r#"
PLBASE="/usr/lib/swi-prolog";
PLLIBDIR="/usr/lib/swi-prolog/library";
PLVERSION="90100";
"#;
    assert_eq!(
        parse_pllibdir(text).as_deref(),
        Some("/usr/lib/swi-prolog/library")
    );
}

#[test]
fn parse_pllibdir_handles_bare_form() {
    let text = "PLBASE=/usr/lib/swipl\nPLLIBDIR=/usr/lib/swipl/library\n";
    assert_eq!(parse_pllibdir(text).as_deref(), Some("/usr/lib/swipl/library"));
}

#[test]
fn parse_pllibdir_returns_none_when_var_missing() {
    let text = "PLBASE=/foo\nUNRELATED=/bar\n";
    assert_eq!(parse_pllibdir(text), None);
}

/// Walking a tree with `.pl`, `.qlf`, and `.html` siblings emits only the
/// `.pl` files.
#[test]
fn walker_emits_only_pl_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    let library = root.join("library");
    fs::create_dir_all(&library).unwrap();
    fs::write(library.join("lists.pl"), b"% stub").unwrap();
    fs::write(library.join("lists.qlf"), b"compiled").unwrap();
    fs::write(library.join("README.md"), b"doc").unwrap();
    let nested = library.join("http");
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("http_dispatch.pl"), b"% stub").unwrap();

    let dep = ExternalDepRoot {
        module_path: "swipl/library".to_string(),
        version: "local".to_string(),
        root: library,
        ecosystem: ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    let walked = walk_prolog_tree(&dep);
    let names: Vec<&str> = walked
        .iter()
        .map(|w| w.relative_path.as_str())
        .collect();

    assert!(names.iter().any(|n| n.ends_with("lists.pl")));
    assert!(names.iter().any(|n| n.ends_with("http_dispatch.pl")));
    assert!(!names.iter().any(|n| n.ends_with(".qlf")));
    assert!(!names.iter().any(|n| n.ends_with("README.md")));
}
