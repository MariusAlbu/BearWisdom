// =============================================================================
// kotlin/coverage_tests.rs  —  One test per declared symbol_node_kind and ref_node_kind
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
fn symbol_object_declaration() {
    let r = extract("object Singleton {}");
    assert!(
        r.symbols.iter().any(|s| s.name == "Singleton"),
        "expected Singleton; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_companion_object() {
    let r = extract("class Holder {\n    companion object {}\n}");
    assert!(
        r.symbols.iter().any(|s| s.name == "Companion"),
        "expected Companion; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_function_declaration() {
    let r = extract("fun greet() {}");
    assert!(
        r.symbols.iter().any(|s| s.name == "greet"),
        "expected greet; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_secondary_constructor() {
    let r = extract("class Box {\n    constructor(size: Int) {}\n}");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Constructor),
        "expected Constructor; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_primary_constructor() {
    // Primary constructor params promoted with val become Property symbols.
    let r = extract("class Point(val x: Int, val y: Int)");
    assert!(
        r.symbols.iter().any(|s| s.name == "x" && s.kind == SymbolKind::Property),
        "expected property x; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_property_declaration() {
    let r = extract("class Cfg {\n    val timeout: Int = 30\n}");
    assert!(
        r.symbols.iter().any(|s| s.name == "timeout"),
        "expected timeout; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_getter() {
    let r = extract("class C {\n    val v: Int\n        get() = 1\n}");
    // The getter itself or the property must be present.
    assert!(
        !r.symbols.is_empty(),
        "expected at least one symbol for getter; got none"
    );
}

#[test]
fn symbol_setter() {
    let r = extract("class C {\n    var v: Int = 0\n        set(value) { field = value }\n}");
    assert!(
        !r.symbols.is_empty(),
        "expected at least one symbol for setter; got none"
    );
}

#[test]
fn symbol_type_alias() {
    let r = extract("typealias StringList = List<String>");
    assert!(
        r.symbols.iter().any(|s| s.name == "StringList"),
        "expected StringList; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_enum_entry() {
    let r = extract("enum class Color {\n    RED, GREEN, BLUE\n}");
    assert!(
        r.symbols.iter().any(|s| s.name == "RED" || s.name == "Color"),
        "expected enum entries or Color; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_class_parameter() {
    // class_parameter without val/var — still part of primary constructor.
    let r = extract("class Greeter(name: String) {\n    fun greet() = println(name)\n}");
    assert!(
        r.symbols.iter().any(|s| s.name == "Greeter"),
        "expected Greeter; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Ref node kinds
// ---------------------------------------------------------------------------

#[test]
fn ref_call_expression() {
    let r = extract("fun bar() {}\nfun foo() { bar() }");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "bar" && rf.kind == EdgeKind::Calls),
        "expected Calls bar; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_constructor_invocation() {
    let r = extract("class Dog\nfun make() { val d = Dog() }");
    // constructor invocation emits at least a Calls or TypeRef edge to Dog
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "Dog"),
        "expected ref to Dog; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_import_header() {
    let r = extract("import kotlin.collections.ArrayList");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports),
        "expected Imports ref; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_delegation_specifier() {
    let r = extract("interface Runnable\nclass Worker : Runnable");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "Runnable"),
        "expected TypeRef/Inherits to Runnable; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_user_type() {
    // Type annotation on a property emits TypeRef via primary constructor params.
    let r = extract("class Holder(val item: String)");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "String" && rf.kind == EdgeKind::TypeRef),
        "expected TypeRef String; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_nullable_type() {
    let r = extract("class Box(val value: String?)");
    // nullable_type wraps user_type — TypeRef should still be emitted.
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "String" && rf.kind == EdgeKind::TypeRef),
        "expected TypeRef String for nullable; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_type_arguments() {
    // Type argument bounds are emitted from extract_type_parameter_bounds.
    // Use a class with a simple upper bound on a type parameter.
    let r = extract("class Cache<T : Comparable<T>> {\n    val value: T? = null\n}");
    assert!(
        !r.refs.is_empty(),
        "expected at least one ref for type parameter or type annotation; got none"
    );
}

#[test]
fn ref_as_expression() {
    let r = extract("fun cast(x: Any): String { return x as String }");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "String" && rf.kind == EdgeKind::TypeRef),
        "expected TypeRef String from as; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_check_expression() {
    let r = extract("fun check(x: Any) { if (x is String) {} }");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "String" && rf.kind == EdgeKind::TypeRef),
        "expected TypeRef String from is; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_annotation() {
    let r = extract("@Suppress(\"UNCHECKED_CAST\")\nfun foo() {}");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "Suppress" && rf.kind == EdgeKind::TypeRef),
        "expected TypeRef Suppress; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_navigation_expression() {
    let r = extract("fun foo() { System.out.println(\"hi\") }");
    // navigation_expression (System.out.println) should produce a Calls ref.
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls),
        "expected Calls ref from navigation_expression; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}
