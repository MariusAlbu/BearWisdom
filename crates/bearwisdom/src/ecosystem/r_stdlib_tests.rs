use super::*;
use std::fs;

// ---------------------------------------------------------------------------
// Source-distro walk tests (unchanged from original)
// ---------------------------------------------------------------------------

fn make_r_src_fixture(root: &Path, packages: &[(&str, &[&str])]) {
    let library = root.join("src").join("library");
    fs::create_dir_all(&library).unwrap();
    for (pkg, files) in packages {
        let pkg_r = library.join(pkg).join("R");
        fs::create_dir_all(&pkg_r).unwrap();
        for fname in *files {
            fs::write(pkg_r.join(fname), "# stub\n").unwrap();
        }
    }
}

#[test]
fn walk_yields_r_files_per_base_package() {
    let tmp = tempfile::tempdir().unwrap();
    make_r_src_fixture(
        tmp.path(),
        &[
            ("base", &["zzz.R", "library.R"]),
            ("stats", &["lm.R"]),
            // Non-base package — should be skipped.
            ("dplyr", &["filter.R"]),
        ],
    );

    let dep = ExternalDepRoot {
        module_path: KIND_SOURCE.into(),
        version: String::new(),
        root: tmp.path().join("src").join("library"),
        ecosystem: TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    let walked = walk_r_tree(&dep);
    let names: Vec<&str> = walked
        .iter()
        .map(|w| w.absolute_path.file_name().unwrap().to_str().unwrap())
        .collect();
    assert!(names.contains(&"zzz.R"));
    assert!(names.contains(&"library.R"));
    assert!(names.contains(&"lm.R"));
    assert!(!names.contains(&"filter.R"), "non-base package must be skipped");
    for w in &walked {
        assert_eq!(w.language, "r");
        assert!(w.relative_path.starts_with("ext:r-stdlib:"));
    }
}

#[test]
fn discover_returns_empty_without_env_var() {
    // Make sure no leftover var from another test leaks in.
    std::env::remove_var("BEARWISDOM_R_SRC");
    std::env::remove_var("R_HOME");
    // Either empty (no R) or non-empty (R found via subprocess) — assert no panic.
    let _ = discover_r_stdlib();
}

#[test]
fn discover_returns_no_source_root_when_path_lacks_src_library() {
    let tmp = tempfile::tempdir().unwrap();
    // Tmp dir exists but has no src/library/ child.
    std::env::set_var("BEARWISDOM_R_SRC", tmp.path());
    let roots = discover_r_stdlib();
    std::env::remove_var("BEARWISDOM_R_SRC");
    // The SOURCE path must not produce a root; installed-R / user-library
    // probes may still fire if a real R is installed on the host machine.
    assert!(
        roots.iter().all(|r| r.module_path != KIND_SOURCE),
        "invalid BEARWISDOM_R_SRC must not produce a KIND_SOURCE root"
    );
}

#[test]
fn discover_returns_one_root_with_valid_r_src() {
    let tmp = tempfile::tempdir().unwrap();
    make_r_src_fixture(tmp.path(), &[("base", &["zzz.R"])]);

    std::env::set_var("BEARWISDOM_R_SRC", tmp.path());
    let roots = discover_r_stdlib();
    std::env::remove_var("BEARWISDOM_R_SRC");

    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].module_path, KIND_SOURCE);
    assert!(roots[0].root.ends_with("src/library") || roots[0].root.ends_with("src\\library"));
}

#[test]
fn walk_returns_empty_when_root_missing() {
    let dep = ExternalDepRoot {
        module_path: KIND_SOURCE.into(),
        version: String::new(),
        root: PathBuf::from("/__no_such_r_src_for_test__/zzz"),
        ecosystem: TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    assert!(walk_r_tree(&dep).is_empty());
}

// ---------------------------------------------------------------------------
// NAMESPACE parsing tests
// ---------------------------------------------------------------------------

const BASE_NAMESPACE: &str = r#"
# base NAMESPACE
export(c, length, nchar, paste, paste0, print, cat, message, warning, stop)
export(`[`, `[[`, `$`, `+`, `-`, `*`, `/`)
S3method(print, default)
S3method(format, Date)
exportClasses(Date)
exportMethods(show)
exportPattern("^[[:alpha:]]")
useDynLib(base, .registration = TRUE)
import(methods)
"#;

#[test]
fn parse_namespace_export_simple_names() {
    let mut out = Vec::new();
    parse_namespace(BASE_NAMESPACE, "base", &mut out);

    let names: Vec<&str> = out.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"c"), "c() must be exported");
    assert!(names.contains(&"paste"), "paste() must be exported");
    assert!(names.contains(&"paste0"), "paste0() must be exported");
    assert!(names.contains(&"length"), "length() must be exported");
    assert!(names.contains(&"nchar"), "nchar() must be exported");
}

#[test]
fn parse_namespace_export_operator_backtick_names() {
    let mut out = Vec::new();
    parse_namespace(BASE_NAMESPACE, "base", &mut out);

    let names: Vec<&str> = out.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"["), "backtick-quoted `[` must be extracted");
    assert!(names.contains(&"[["), "backtick-quoted `[[` must be extracted");
    assert!(names.contains(&"$"), "backtick-quoted `$` must be extracted");
    assert!(names.contains(&"+"), "backtick-quoted `+` must be extracted");
}

#[test]
fn parse_namespace_s3method_emits_generic() {
    let mut out = Vec::new();
    parse_namespace(BASE_NAMESPACE, "base", &mut out);

    let names: Vec<&str> = out.iter().map(|s| s.name.as_str()).collect();
    // print and format are the generic names from S3method(print, default) and
    // S3method(format, Date).
    assert!(names.contains(&"print"), "S3method generic print must be emitted");
    assert!(names.contains(&"format"), "S3method generic format must be emitted");
}

#[test]
fn parse_namespace_export_classes() {
    let mut out = Vec::new();
    parse_namespace(BASE_NAMESPACE, "base", &mut out);

    let class_sym = out.iter().find(|s| s.name == "Date" && matches!(s.kind, SymbolKind::Class));
    assert!(class_sym.is_some(), "exportClasses(Date) must emit a Class symbol");
}

#[test]
fn parse_namespace_export_methods() {
    let mut out = Vec::new();
    parse_namespace(BASE_NAMESPACE, "base", &mut out);

    let names: Vec<&str> = out.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"show"), "exportMethods(show) must emit show");
}

#[test]
fn parse_namespace_skips_comments_and_import_directives() {
    let ns = "# top comment\nimportFrom(methods, setClass)\nuseDynLib(foo)\nexport(myFunc)\n";
    let mut out = Vec::new();
    parse_namespace(ns, "testpkg", &mut out);

    let names: Vec<&str> = out.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["myFunc"], "only export() symbols must appear; import/useDynLib are skipped");
}

#[test]
fn parse_namespace_qualified_name_includes_package() {
    let ns = "export(mean)\n";
    let mut out = Vec::new();
    parse_namespace(ns, "base", &mut out);

    assert_eq!(out.len(), 1);
    assert_eq!(out[0].name, "mean");
    assert_eq!(out[0].qualified_name, "base::mean");
}

#[test]
fn parse_namespace_visibility_is_public() {
    let ns = "export(sum)\n";
    let mut out = Vec::new();
    parse_namespace(ns, "base", &mut out);

    assert!(matches!(out[0].visibility, Some(Visibility::Public)));
}

// ---------------------------------------------------------------------------
// NAMESPACE-based synthesis from library directory
// ---------------------------------------------------------------------------

fn make_installed_r_fixture(root: &Path, packages: &[(&str, &str)]) {
    for (pkg, ns_content) in packages {
        let pkg_dir = root.join(pkg);
        fs::create_dir_all(&pkg_dir).unwrap();
        // DESCRIPTION is the per-package marker the walker uses to recognise
        // a real package directory (base has no NAMESPACE, every package has
        // DESCRIPTION).
        fs::write(pkg_dir.join("DESCRIPTION"), format!("Package: {pkg}\n")).unwrap();
        fs::write(pkg_dir.join("NAMESPACE"), ns_content).unwrap();
    }
}

#[test]
fn synthesize_from_namespace_returns_parsed_file() {
    let tmp = tempfile::tempdir().unwrap();
    make_installed_r_fixture(
        tmp.path(),
        &[
            ("base", "export(c, length, sum)\nS3method(print, default)\n"),
            ("stats", "export(lm, glm, t.test)\n"),
        ],
    );

    let files = synthesize_from_namespace(tmp.path());
    assert_eq!(files.len(), 1, "should produce exactly one synthetic ParsedFile");

    let pf = &files[0];
    assert_eq!(pf.language, "r");
    assert!(pf.path.starts_with("ext:r-stdlib:"));

    let names: Vec<&str> = pf.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"c"));
    assert!(names.contains(&"length"));
    assert!(names.contains(&"sum"));
    assert!(names.contains(&"print"), "S3method generic must be included");
    assert!(names.contains(&"lm"));
    assert!(names.contains(&"glm"));
    assert!(names.contains(&"t.test"));
}

#[test]
fn synthesize_from_namespace_returns_empty_when_no_packages_present() {
    let tmp = tempfile::tempdir().unwrap();
    // library root exists but has no base/NAMESPACE etc.
    let files = synthesize_from_namespace(tmp.path());
    assert!(files.is_empty(), "no NAMESPACE files means no synthetic output");
}

#[test]
fn synthesize_from_namespace_symbol_and_origin_counts_match() {
    let tmp = tempfile::tempdir().unwrap();
    make_installed_r_fixture(
        tmp.path(),
        &[("base", "export(a, b, c)\n")],
    );

    let files = synthesize_from_namespace(tmp.path());
    assert_eq!(files.len(), 1);
    let pf = &files[0];
    // ParsedFile invariant: symbol_origin_languages and symbol_from_snippet
    // must be parallel to symbols.
    assert_eq!(pf.symbol_origin_languages.len(), pf.symbols.len());
    assert_eq!(pf.symbol_from_snippet.len(), pf.symbols.len());
}

// ---------------------------------------------------------------------------
// Installed-R discovery via R_HOME
// ---------------------------------------------------------------------------

#[test]
fn discover_uses_r_home_when_library_base_description_present() {
    let tmp = tempfile::tempdir().unwrap();
    // Simulate minimal installed R: library/base/DESCRIPTION must exist
    // (base has no NAMESPACE — base is hardcoded in R itself).
    let base_dir = tmp.path().join("library").join("base");
    fs::create_dir_all(&base_dir).unwrap();
    fs::write(base_dir.join("DESCRIPTION"), "Package: base\n").unwrap();

    std::env::remove_var("BEARWISDOM_R_SRC");
    std::env::set_var("R_HOME", tmp.path());
    let roots = discover_r_stdlib();
    std::env::remove_var("R_HOME");

    assert!(!roots.is_empty(), "system library should produce a root");
    let sys_root = roots.iter().find(|r| r.module_path == KIND_NAMESPACE)
        .expect("KIND_NAMESPACE root expected for installed R");
    assert!(
        sys_root.root.ends_with("library"),
        "system root must point at the library/ subdirectory, got: {}",
        sys_root.root.display()
    );
}

#[test]
fn discover_does_not_attribute_invalid_r_home_to_a_root() {
    let tmp = tempfile::tempdir().unwrap();
    // R_HOME exists but library/base/DESCRIPTION is absent.
    std::env::remove_var("BEARWISDOM_R_SRC");
    std::env::set_var("R_HOME", tmp.path());
    let roots = discover_r_stdlib();
    std::env::remove_var("R_HOME");

    // No root should point at the invalid R_HOME's library subdir.
    let bad_library = tmp.path().join("library");
    assert!(
        roots.iter().all(|r| r.root != bad_library),
        "invalid R_HOME must not produce a root pointing at its library/ subdir; \
         user-library probes may still fire on machines with a real R install"
    );
}

#[test]
fn source_distro_takes_priority_over_r_home() {
    let tmp = tempfile::tempdir().unwrap();
    // Set up a valid source distro.
    make_r_src_fixture(tmp.path(), &[("base", &["zzz.R"])]);
    // Also set up a fake R_HOME so both would match.
    let r_home = tmp.path().join("fake_r_home");
    let base_dir = r_home.join("library").join("base");
    fs::create_dir_all(&base_dir).unwrap();
    fs::write(base_dir.join("DESCRIPTION"), "Package: base\n").unwrap();

    std::env::set_var("BEARWISDOM_R_SRC", tmp.path());
    std::env::set_var("R_HOME", &r_home);
    let roots = discover_r_stdlib();
    std::env::remove_var("BEARWISDOM_R_SRC");
    std::env::remove_var("R_HOME");

    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].module_path, KIND_SOURCE,
        "source distro must win over installed R when both are present");
}
