use std::fs;

use tempfile::TempDir;

use super::*;

const TEST_ECOSYSTEM: &str = "freepascal-runtime";

fn make_dep(root: &std::path::Path, module: &str) -> ExternalDepRoot {
    ExternalDepRoot {
        module_path: module.to_string(),
        version: String::new(),
        root: root.to_path_buf(),
        ecosystem: TEST_ECOSYSTEM,
        package_id: None,
        requested_imports: Vec::new(),
    }
}

#[test]
fn symbol_index_scans_pas_and_pp_but_not_non_pascal_files() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("lcl");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("forms.pas"), "unit Forms;\ninterface\nimplementation\n").unwrap();
    fs::write(root.join("buttons.pp"), "unit Buttons;\ninterface\nimplementation\n").unwrap();
    fs::write(root.join("README.md"), "docs\n").unwrap();

    let dep = make_dep(&root, "lcl");
    let idx = build_pascal_symbol_index(&[dep]);
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

    let dep = make_dep(&root, "lcl");
    let idx = build_pascal_symbol_index(&[dep]);
    assert!(idx.locate("lcl", "forms").is_some(), "top-level forms.pas must be indexed");
    assert!(idx.locate("lcl", "testforms").is_none(), "tests/ dir must be skipped");
    assert!(idx.locate("lcl", "hello").is_none(), "examples/ dir must be skipped");
    assert!(idx.locate("lcl", "demo").is_none(), "demos/ dir must be skipped");
}

#[test]
fn symbol_index_registers_unit_name() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("sysutils.pp"), "unit SysUtils;\ninterface\nimplementation\n").unwrap();
    let dep = make_dep(tmp.path(), "fpc-rtl-objpas");
    let idx = build_pascal_symbol_index(&[dep]);
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
    let idx = build_pascal_symbol_index(&[dep]);

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
    let idx = build_pascal_symbol_index(&[dep]);

    assert!(idx.locate("mod", "ifaceproc").is_some());
    assert!(
        idx.locate("mod", "implonlyproc").is_none(),
        "names declared after `implementation` must not be indexed"
    );
}

#[test]
fn symbol_index_scans_inc_fragments_without_unit_wrapper() {
    // `.inc` fragments carry no `unit`/`interface` header — the RTL splices
    // them into a parent unit via `{$I}`. Regression coverage for the
    // extraction gap: system.pp's interface is just `{$I systemh.inc}`, so
    // the real declarations (TObject, GetMem, ...) only surface if the
    // fragment itself is scanned as a standalone declaration list.
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join("systemh.inc"),
        "type\n  TObject = class\n  end;\nprocedure GetMem(var p: Pointer; n: SizeInt);\n",
    )
    .unwrap();
    fs::write(
        tmp.path().join("system.pp"),
        "unit System;\ninterface\n{$I systemh.inc}\nimplementation\nend.\n",
    )
    .unwrap();
    let dep = make_dep(tmp.path(), "fpc-rtl-inc");
    let idx = build_pascal_symbol_index(&[dep]);

    // system.pp still contributes its own unit name.
    assert!(idx.locate("fpc-rtl-inc", "system").is_some());
    // The fragment's own declarations are locatable — this is the gap fix.
    assert!(idx.locate("fpc-rtl-inc", "tobject").is_some(), "TObject must resolve from the .inc fragment");
    assert!(idx.locate("fpc-rtl-inc", "getmem").is_some(), "GetMem must resolve from the .inc fragment");
}

#[test]
fn symbol_index_inc_fragment_stops_at_implementation() {
    // A fragment that happens to carry its own `implementation` marker
    // (uncommon, but the state machine must still gate on it) stops
    // harvesting there, same as a full unit.
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join("frag.inc"),
        "procedure IfaceProc;\nimplementation\nprocedure ImplOnlyProc;\n",
    )
    .unwrap();
    let dep = make_dep(tmp.path(), "mod");
    let idx = build_pascal_symbol_index(&[dep]);

    assert!(idx.locate("mod", "ifaceproc").is_some());
    assert!(idx.locate("mod", "implonlyproc").is_none());
}

#[test]
fn extract_decl_ident_recognises_keywords() {
    assert_eq!(extract_decl_ident("procedure dosomething(x: integer)"), Some("dosomething"));
    assert_eq!(extract_decl_ident("function getvalue: string"), Some("getvalue"));
    assert_eq!(extract_decl_ident("type tmyclass = class"), Some("tmyclass"));
    assert_eq!(extract_decl_ident("var globalflag: boolean"), Some("globalflag"));
    assert_eq!(extract_decl_ident("const max_size = 100"), Some("max_size"));
    // Non-declaration lines return None.
    assert_eq!(extract_decl_ident("begin"), None);
    assert_eq!(extract_decl_ident("end."), None);
    assert_eq!(extract_decl_ident("uses sysutils;"), None);
}
