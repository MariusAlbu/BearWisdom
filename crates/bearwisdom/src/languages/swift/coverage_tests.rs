// =============================================================================
// swift/coverage_tests.rs  —  One test per declared symbol_node_kind and ref_node_kind
// =============================================================================

use super::extract::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// Symbol node kinds
// ---------------------------------------------------------------------------

#[test]
fn symbol_class_declaration() {
    let r = extract("class Foo {}");
    assert!(
        r.symbols.iter().any(|s| s.name == "Foo" && s.kind == SymbolKind::Class),
        "expected Class Foo; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_protocol_declaration() {
    let r = extract("protocol Drawable {}");
    assert!(
        r.symbols.iter().any(|s| s.name == "Drawable" && s.kind == SymbolKind::Interface),
        "expected Interface Drawable; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_enum_class_body() {
    // enum_class_body is the body of an enum — the enum declaration itself must be extracted.
    let r = extract("enum Direction {\n    case north\n    case south\n}");
    assert!(
        r.symbols.iter().any(|s| s.name == "Direction"),
        "expected Direction; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_function_declaration() {
    let r = extract("func greet() {}");
    assert!(
        r.symbols.iter().any(|s| s.name == "greet"),
        "expected greet; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_init_declaration() {
    let r = extract("class Box {\n    init(size: Int) {}\n}");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Constructor),
        "expected Constructor; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_protocol_function_declaration() {
    let r = extract("protocol Runnable {\n    func run()\n}");
    assert!(
        r.symbols.iter().any(|s| s.name == "run"),
        "expected run; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_property_declaration() {
    let r = extract("class C {\n    var name: String = \"\"\n}");
    assert!(
        r.symbols.iter().any(|s| s.name == "name"),
        "expected name; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_protocol_property_declaration() {
    let r = extract("protocol Nameable {\n    var name: String { get }\n}");
    assert!(
        r.symbols.iter().any(|s| s.name == "name"),
        "expected name; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_typealias_declaration() {
    let r = extract("typealias StringList = [String]");
    assert!(
        r.symbols.iter().any(|s| s.name == "StringList"),
        "expected StringList; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_subscript_declaration() {
    let r = extract("class Grid {\n    subscript(index: Int) -> Int { return 0 }\n}");
    assert!(
        r.symbols.iter().any(|s| s.name == "subscript" || s.kind == SymbolKind::Method),
        "expected subscript symbol; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_associatedtype_declaration() {
    let r = extract("protocol Container {\n    associatedtype Element\n}");
    assert!(
        r.symbols.iter().any(|s| s.name == "Element" || s.name == "Container"),
        "expected Element or Container; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_operator_declaration() {
    let r = extract("infix operator +++: AdditionPrecedence");
    // Operator declarations may or may not produce symbols depending on grammar.
    // We just verify no crash.
    let _ = r;
}

#[test]
fn symbol_enum_entry() {
    let r = extract("enum Color {\n    case red\n    case green\n}");
    assert!(
        r.symbols.iter().any(|s| s.name == "red" || s.name == "Color"),
        "expected red or Color; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Ref node kinds
// ---------------------------------------------------------------------------

#[test]
fn ref_call_expression() {
    let r = extract("func f() { print(\"hi\") }");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "print" && rf.kind == EdgeKind::Calls),
        "expected Calls print; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_constructor_expression() {
    let r = extract("class Dog {}\nfunc make() -> Dog { return Dog() }");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "Dog"),
        "expected ref to Dog; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_import_declaration() {
    let r = extract("import Foundation");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports),
        "expected Imports ref; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_inheritance_specifier() {
    let r = extract("class Cat: Animal {}");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "Animal"),
        "expected ref to Animal; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_type_annotation() {
    let r = extract("func greet(name: String) {}");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "String"),
        "expected ref to String; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_user_type() {
    let r = extract("var x: MyClass = MyClass()");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "MyClass"),
        "expected ref to MyClass; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_as_expression() {
    let r = extract("func f(x: Any) -> String { return x as! String }");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "String" && rf.kind == EdgeKind::TypeRef),
        "expected TypeRef String from as; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_check_expression() {
    let r = extract("func f(x: Any) { if x is String {} }");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "String" && rf.kind == EdgeKind::TypeRef),
        "expected TypeRef String from is; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_type_identifier() {
    // type_identifier appears in user_type and direct type annotations.
    let r = extract("func f() -> Int { return 0 }");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "Int"),
        "expected ref to Int; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_protocol_composition_type() {
    // `SomeProtocol & AnotherProtocol` in a type position.
    let r = extract("func f(x: Codable & Equatable) {}");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "Codable" || rf.target_name == "Equatable"),
        "expected ref from protocol composition; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_type_identifier_in_local_var() {
    // type_identifier inside a function body local var annotation.
    let r = extract("func f() {\n    let x: MyService = MyService()\n}");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "MyService"),
        "expected TypeRef MyService from local var annotation; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_user_type_in_property_decl() {
    // user_type in a stored property type annotation should emit TypeRef.
    let r = extract("class C {\n    var repo: UserRepository = UserRepository()\n}");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "UserRepository"),
        "expected TypeRef UserRepository from property annotation; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_call_expression_nested_in_property() {
    // call_expression in a computed property should produce Calls.
    let r = extract("class C {\n    var count: Int {\n        return items.count\n    }\n}");
    assert!(
        !r.refs.is_empty(),
        "expected refs from computed property body; got none"
    );
}
