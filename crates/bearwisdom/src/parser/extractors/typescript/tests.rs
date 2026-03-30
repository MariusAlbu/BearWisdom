use super::*;
use crate::types::{EdgeKind, SymbolKind};

fn sym(source: &str) -> Vec<ExtractedSymbol> { extract(source, false).symbols }
fn refs(source: &str) -> Vec<ExtractedRef>    { extract(source, false).refs }

#[test]
fn extracts_class() {
    let src = "export class UserService {}";
    let symbols = sym(src);
    let s = symbols.iter().find(|s| s.name == "UserService").unwrap();
    assert_eq!(s.kind, SymbolKind::Class);
}

#[test]
fn extracts_interface() {
    let src = "interface IRepository { save(): void; }";
    let symbols = sym(src);
    let i = symbols.iter().find(|s| s.name == "IRepository").unwrap();
    assert_eq!(i.kind, SymbolKind::Interface);
}

#[test]
fn extracts_function() {
    let src = "function fetchUsers(): Promise<User[]> { return []; }";
    let symbols = sym(src);
    let f = symbols.iter().find(|s| s.name == "fetchUsers").unwrap();
    assert_eq!(f.kind, SymbolKind::Function);
}

#[test]
fn extracts_method() {
    let src = "class Svc { async getById(id: number) {} }";
    let symbols = sym(src);
    let m = symbols.iter().find(|s| s.name == "getById").unwrap();
    assert_eq!(m.kind, SymbolKind::Method);
}

#[test]
fn extracts_type_alias() {
    let src = "type UserId = string;";
    let symbols = sym(src);
    let t = symbols.iter().find(|s| s.name == "UserId").unwrap();
    assert_eq!(t.kind, SymbolKind::TypeAlias);
}

#[test]
fn extracts_extends_as_inherits() {
    let src = "class Foo extends Bar {}";
    let r = refs(src);
    assert!(r.iter().any(|r| r.target_name == "Bar" && r.kind == EdgeKind::Inherits),
        "refs: {r:?}");
}

#[test]
fn extracts_import() {
    let src = r#"import { CatalogService } from "./catalog";"#;
    let r = refs(src);
    assert!(r.iter().any(|r| r.target_name == "CatalogService"), "refs: {r:?}");
    let imp = r.iter().find(|r| r.target_name == "CatalogService").unwrap();
    assert_eq!(imp.module, Some("./catalog".to_string()));
}

#[test]
fn extracts_call() {
    let src = "function run() { fetchData(); }";
    let r = refs(src);
    let calls: Vec<_> = r.iter().filter(|r| r.kind == EdgeKind::Calls).collect();
    assert!(calls.iter().any(|r| r.target_name == "fetchData"), "calls: {calls:?}");
}

#[test]
fn qualified_name_includes_class() {
    let src = "class Catalog { list(): void {} }";
    let symbols = sym(src);
    let m = symbols.iter().find(|s| s.name == "list").unwrap();
    assert_eq!(m.qualified_name, "Catalog.list");
}

#[test]
fn does_not_panic_on_malformed_source() {
    let src = "class { !!! broken @@@ }";
    let _ = extract(src, false);
}
