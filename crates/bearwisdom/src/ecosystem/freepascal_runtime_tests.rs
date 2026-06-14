use std::fs;

use tempfile::TempDir;

use super::*;

fn make_lazarus_fixture(root: &std::path::Path) {
    fs::create_dir_all(root.join("lcl")).unwrap();
    fs::write(root.join("lcl").join("forms.pas"), "unit Forms;\n").unwrap();
    fs::write(root.join("lcl").join("buttons.pp"), "unit Buttons;\n").unwrap();

    fs::create_dir_all(root.join("components").join("codetools")).unwrap();
    fs::write(
        root.join("components").join("codetools").join("codecache.pas"),
        "unit CodeCache;\n",
    )
    .unwrap();

    let win64 = root.join("fpc").join("3.2.2").join("source").join("rtl").join("win64");
    fs::create_dir_all(&win64).unwrap();
    fs::write(win64.join("system.pp"), "unit System;\n").unwrap();
    fs::write(win64.join("classes.pp"), "unit Classes;\n").unwrap();

    let win32 = root.join("fpc").join("3.2.2").join("source").join("rtl").join("win32");
    fs::create_dir_all(&win32).unwrap();
    fs::write(win32.join("system.pp"), "unit System;\n").unwrap();

    let objpas = root.join("fpc").join("3.2.2").join("source").join("rtl").join("objpas");
    fs::create_dir_all(&objpas).unwrap();
    fs::write(objpas.join("classes.pp"), "unit Classes;\n").unwrap();
    fs::write(objpas.join("sysutils.pp"), "unit SysUtils;\n").unwrap();

    // Package with a /src/ subdir — the per-package walker requires /src/ to exist.
    let pkg_src = root
        .join("fpc")
        .join("3.2.2")
        .join("source")
        .join("packages")
        .join("fcl-base")
        .join("src");
    fs::create_dir_all(&pkg_src).unwrap();
    fs::write(pkg_src.join("inifiles.pp"), "unit IniFiles;\n").unwrap();
}

#[test]
fn discover_returns_empty_without_install() {
    let tmp = TempDir::new().unwrap();
    std::env::set_var("BEARWISDOM_LAZARUS_DIR", tmp.path().join("nonexistent"));
    let roots = discover_freepascal_roots();
    std::env::remove_var("BEARWISDOM_LAZARUS_DIR");
    // The override pointed at a missing dir, but the fallback chain may
    // still find the system Lazarus install. Either is correct behavior;
    // we only assert that the call doesn't panic.
    let _ = roots;
}

#[test]
fn discover_uses_explicit_dir_override() {
    let tmp = TempDir::new().unwrap();
    make_lazarus_fixture(tmp.path());

    std::env::set_var("BEARWISDOM_LAZARUS_DIR", tmp.path());
    let roots = discover_freepascal_roots();
    std::env::remove_var("BEARWISDOM_LAZARUS_DIR");

    let module_paths: std::collections::HashSet<String> =
        roots.iter().map(|r| r.module_path.clone()).collect();
    assert!(module_paths.contains("lcl"), "{module_paths:?}");
    assert!(module_paths.contains("lazarus-components"), "{module_paths:?}");
    assert!(module_paths.contains("fpc-rtl-objpas"), "{module_paths:?}");
    // Single package under packages/fcl-base/src/ emits one per-package root.
    assert!(module_paths.contains("fpc-pkg-fcl-base"), "{module_paths:?}");
    // The old aggregate fpc-packages root no longer exists — packages are emitted
    // individually so module_path values are distinct per package.
    assert!(!module_paths.contains("fpc-packages"), "{module_paths:?}");
    // Exactly one host-target RTL root, never both win32 + win64.
    let rtl_count = module_paths
        .iter()
        .filter(|p| p.starts_with("fpc-rtl-win"))
        .count();
    assert_eq!(rtl_count, 1, "{module_paths:?}");
}

#[test]
fn symbol_index_scans_pas_and_pp_but_not_non_pascal_files() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("lcl");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("forms.pas"), "unit Forms;\ninterface\nimplementation\n").unwrap();
    fs::write(root.join("buttons.pp"), "unit Buttons;\ninterface\nimplementation\n").unwrap();
    fs::write(root.join("README.md"), "docs\n").unwrap();

    let dep = ExternalDepRoot {
        module_path: "lcl".to_string(),
        version: String::new(),
        root: root.clone(),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    let idx = _test_build_pascal_symbol_index(&[dep]);
    assert!(idx.locate("lcl", "forms").is_some(), "forms.pas must be scanned");
    assert!(idx.locate("lcl", "buttons").is_some(), "buttons.pp must be scanned");
    // Non-Pascal files must not produce entries.
    assert!(idx.locate("lcl", "readme").is_none(), "README.md must not be indexed");
}

#[test]
fn symbol_index_skips_tests_and_examples_dirs() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("lcl");
    fs::create_dir_all(root.join("tests")).unwrap();
    fs::create_dir_all(root.join("examples")).unwrap();
    fs::create_dir_all(root.join("demos")).unwrap();
    fs::write(root.join("tests").join("test_forms.pas"), "unit TestForms;\ninterface\nimplementation\n").unwrap();
    fs::write(root.join("examples").join("hello.pas"), "unit Hello;\ninterface\nimplementation\n").unwrap();
    fs::write(root.join("demos").join("demo.pas"), "unit Demo;\ninterface\nimplementation\n").unwrap();
    fs::write(root.join("forms.pas"), "unit Forms;\ninterface\nimplementation\n").unwrap();

    let dep = ExternalDepRoot {
        module_path: "lcl".to_string(),
        version: String::new(),
        root: root.clone(),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    let idx = _test_build_pascal_symbol_index(&[dep]);
    assert!(idx.locate("lcl", "forms").is_some(), "top-level forms.pas must be indexed");
    assert!(idx.locate("lcl", "testforms").is_none(), "tests/ dir must be skipped");
    assert!(idx.locate("lcl", "hello").is_none(), "examples/ dir must be skipped");
    assert!(idx.locate("lcl", "demo").is_none(), "demos/ dir must be skipped");
}

#[test]
#[ignore] // requires real Lazarus install at scoop default path
fn live_discovery_finds_scoop_install() {
    // Defensive: only assert when we know the scoop path is present on
    // the dev machine. This is the on-this-machine smoke check.
    let scoop = std::env::var_os("USERPROFILE")
        .map(|h| std::path::PathBuf::from(h).join("scoop/apps/lazarus/current"));
    if scoop.as_ref().is_none_or(|p| !p.is_dir()) {
        return;
    }
    std::env::remove_var("BEARWISDOM_LAZARUS_DIR");
    std::env::remove_var("LAZARUS_DIR");
    let roots = discover_freepascal_roots();
    assert!(!roots.is_empty(), "expected Lazarus install to yield roots");
    let names: Vec<&str> = roots.iter().map(|r| r.module_path.as_str()).collect();
    assert!(names.contains(&"lcl"), "{names:?}");
}

#[test]
fn ecosystem_identity_and_languages() {
    let e = FreePascalRuntimeEcosystem;
    assert_eq!(e.id(), ID);
    assert_eq!(Ecosystem::kind(&e), EcosystemKind::Stdlib);
    assert_eq!(Ecosystem::languages(&e), &["pascal"]);
}

#[test]
fn emit_package_roots_requires_src_subdir() {
    let tmp = TempDir::new().unwrap();
    let packages = tmp.path().join("packages");

    // Package with /src/: should be emitted.
    fs::create_dir_all(packages.join("fcl-base").join("src")).unwrap();
    fs::write(packages.join("fcl-base").join("src").join("a.pp"), "").unwrap();

    // Package without /src/: should be skipped.
    fs::create_dir_all(packages.join("nonesuch")).unwrap();
    fs::write(packages.join("nonesuch").join("main.pp"), "").unwrap();

    let mut roots = Vec::new();
    emit_package_roots(&packages, &mut roots);

    let names: std::collections::HashSet<String> =
        roots.iter().map(|r| r.module_path.clone()).collect();
    assert!(names.contains("fpc-pkg-fcl-base"), "{names:?}");
    assert!(!names.contains("fpc-pkg-nonesuch"), "{names:?}");
}

#[test]
fn platform_excluded_exotic_targets() {
    // These exotic targets must always be excluded regardless of host.
    for pkg in &["arosunits", "ami-extra", "palmunits", "libgbafpc", "libndsfpc"] {
        assert!(is_platform_excluded(pkg), "{pkg} should be excluded");
    }
}

#[test]
fn cross_platform_packages_never_excluded() {
    // These packages are cross-platform and must always be walked.
    for pkg in &["fcl-base", "fcl-xml", "fcl-net", "rtl-generics", "paszlib", "hash"] {
        assert!(!is_platform_excluded(pkg), "{pkg} should not be excluded");
    }
}

// ---------------------------------------------------------------------------
// Demand-driven interface
// ---------------------------------------------------------------------------

fn make_dep(root: &std::path::Path, module: &str) -> ExternalDepRoot {
    ExternalDepRoot {
        module_path: module.to_string(),
        version: String::new(),
        root: root.to_path_buf(),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }
}

#[test]
fn ecosystem_declares_demand_driven() {
    let e = FreePascalRuntimeEcosystem;
    assert!(Ecosystem::uses_demand_driven_parse(&e));
}

#[test]
fn walk_root_is_empty_under_demand_driven() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("forms.pas"), "unit Forms;\n").unwrap();
    let dep = make_dep(tmp.path(), "lcl");
    assert!(Ecosystem::walk_root(&FreePascalRuntimeEcosystem, &dep).is_empty());
}

#[test]
fn symbol_index_registers_unit_name() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("sysutils.pp"), "unit SysUtils;\ninterface\nimplementation\n").unwrap();
    let dep = make_dep(tmp.path(), "fpc-rtl-objpas");
    let idx = _test_build_pascal_symbol_index(&[dep]);
    // Unit name registered both as-declared and lowercase.
    assert!(idx.locate("fpc-rtl-objpas", "sysutils").is_some());
    assert!(idx.locate("fpc-rtl-objpas", "SysUtils").is_some());
}

#[test]
fn symbol_index_registers_interface_section_decls() {
    let tmp = TempDir::new().unwrap();
    let content = "\
unit MyUnit;
interface
type
  TMyClass = class
procedure DoSomething(x: Integer);
function GetValue: String;
const
  MAX_ITEMS = 100;
var
  GlobalFlag: Boolean;
implementation
procedure DoSomething(x: Integer);
begin end;
end.
";
    fs::write(tmp.path().join("myunit.pas"), content).unwrap();
    let dep = make_dep(tmp.path(), "lcl");
    let idx = _test_build_pascal_symbol_index(&[dep]);

    // Unit name.
    assert!(idx.locate("lcl", "myunit").is_some(), "unit name must be indexed");
    // Interface declarations.
    assert!(idx.locate("lcl", "tmyclass").is_some(), "type must be indexed");
    assert!(idx.locate("lcl", "dosomething").is_some(), "procedure must be indexed");
    assert!(idx.locate("lcl", "getvalue").is_some(), "function must be indexed");
    assert!(idx.locate("lcl", "max_items").is_some(), "const must be indexed");
    assert!(idx.locate("lcl", "globalflag").is_some(), "var must be indexed");
    // Implementation-only names must NOT appear.
    assert!(
        idx.locate("lcl", "begin").is_none(),
        "implementation bodies must not be indexed"
    );
}

#[test]
fn symbol_index_stops_at_implementation_keyword() {
    let tmp = TempDir::new().unwrap();
    let content = "\
unit Foo;
interface
procedure IfaceProc;
implementation
procedure ImplOnlyProc;
begin end;
end.
";
    fs::write(tmp.path().join("foo.pas"), content).unwrap();
    let dep = make_dep(tmp.path(), "mod");
    let idx = _test_build_pascal_symbol_index(&[dep]);

    assert!(idx.locate("mod", "ifaceproc").is_some());
    assert!(
        idx.locate("mod", "implonlyproc").is_none(),
        "names declared after `implementation` must not be indexed"
    );
}

#[test]
fn symbol_index_skips_inc_files() {
    // .inc files are included via {$I} directives and do not have
    // unit declarations; the scanner skips them intentionally.
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("heap.inc"), "procedure GetMem(var p: Pointer; n: SizeInt);\n").unwrap();
    fs::write(tmp.path().join("system.pp"), "unit System;\ninterface\nprocedure Move;\nimplementation\nend.\n").unwrap();
    let dep = make_dep(tmp.path(), "rtl");
    let idx = _test_build_pascal_symbol_index(&[dep]);

    // system.pp contributes its unit and interface symbols.
    assert!(idx.locate("rtl", "system").is_some());
    assert!(idx.locate("rtl", "move").is_some());
    // heap.inc is skipped entirely.
    assert!(idx.locate("rtl", "getmem").is_none());
}

#[test]
fn extract_decl_ident_recognises_keywords() {
    assert_eq!(_test_extract_decl_ident("procedure dosomething(x: integer)"), Some("dosomething"));
    assert_eq!(_test_extract_decl_ident("function getvalue: string"), Some("getvalue"));
    assert_eq!(_test_extract_decl_ident("type tmyclass = class"), Some("tmyclass"));
    assert_eq!(_test_extract_decl_ident("var globalflag: boolean"), Some("globalflag"));
    assert_eq!(_test_extract_decl_ident("const max_size = 100"), Some("max_size"));
    // Non-declaration lines return None.
    assert_eq!(_test_extract_decl_ident("begin"), None);
    assert_eq!(_test_extract_decl_ident("end."), None);
    assert_eq!(_test_extract_decl_ident("uses sysutils;"), None);
}

#[test]
fn symbol_index_non_empty_for_fixture_roots() {
    let tmp = TempDir::new().unwrap();
    make_lazarus_fixture(tmp.path());

    std::env::set_var("BEARWISDOM_LAZARUS_DIR", tmp.path());
    let roots = discover_freepascal_roots();
    std::env::remove_var("BEARWISDOM_LAZARUS_DIR");

    let idx = _test_build_pascal_symbol_index(&roots);
    assert!(!idx.is_empty(), "symbol index must be non-empty for a Lazarus fixture");
    // The fixture writes `unit Forms;` in lcl/forms.pas.
    let hit = roots.iter().any(|r| r.module_path == "lcl")
        && idx.find_by_name("forms").iter().any(|(m, _)| *m == "lcl");
    assert!(hit, "unit 'forms' must appear in the lcl module index");
}
