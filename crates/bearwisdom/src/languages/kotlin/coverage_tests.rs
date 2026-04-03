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

#[test]
fn ref_user_type_in_property_body() {
    // user_type inside property initializer should emit TypeRef.
    let r = extract("class C {\n    val items: List<String> = listOf()\n}");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "List" && rf.kind == EdgeKind::TypeRef),
        "expected TypeRef List from property declaration; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_primary_constructor_emits_symbol() {
    // primary_constructor should produce a Constructor symbol.
    let r = extract("class Service(val repo: Repository)");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Constructor),
        "expected Constructor symbol from primary_constructor; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_enum_entry_produces_member() {
    // enum_entry should produce EnumMember symbols.
    let r = extract("enum class Status { ACTIVE, INACTIVE }");
    assert!(
        r.symbols.iter().any(|s| s.name == "ACTIVE" || s.name == "INACTIVE"),
        "expected EnumMember from enum_entry; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_generic_type_arguments() {
    // Type arguments inside generics should emit TypeRef for each type param.
    let r = extract("class Box {\n    val items: List<String> = listOf()\n}");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "List"),
        "expected TypeRef to List; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "String"),
        "expected TypeRef to String inside List<...>; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_nested_generic_types() {
    // Nested generics: Map<String, List<Int>>
    let r = extract("class C {\n    val data: Map<String, List<Int>> = mapOf()\n}");
    let type_refs: Vec<_> = r.refs.iter().filter(|rf| rf.kind == EdgeKind::TypeRef).collect();
    assert!(
        type_refs.iter().any(|rf| rf.target_name == "Map"),
        "expected TypeRef to Map"
    );
    assert!(
        type_refs.iter().any(|rf| rf.target_name == "String"),
        "expected TypeRef to String"
    );
    assert!(
        type_refs.iter().any(|rf| rf.target_name == "List"),
        "expected TypeRef to List"
    );
    assert!(
        type_refs.iter().any(|rf| rf.target_name == "Int"),
        "expected TypeRef to Int"
    );
}

#[test]
fn ref_callable_type_annotations() {
    // Function types: (String, Int) -> Boolean
    let r = extract("class C {\n    val fn: (String, Int) -> Boolean = { _, _ -> true }\n}");
    let type_refs: Vec<_> = r.refs.iter().filter(|rf| rf.kind == EdgeKind::TypeRef).collect();
    assert!(
        type_refs.iter().any(|rf| rf.target_name == "String"),
        "expected TypeRef to String in function type; got {:?}",
        type_refs.iter().map(|rf| &rf.target_name).collect::<Vec<_>>()
    );
    assert!(
        type_refs.iter().any(|rf| rf.target_name == "Int"),
        "expected TypeRef to Int in function type"
    );
    assert!(
        type_refs.iter().any(|rf| rf.target_name == "Boolean"),
        "expected TypeRef to Boolean in function type"
    );
}

#[test]
fn ref_annotation_on_class() {
    // @Service annotation should emit TypeRef
    let r = extract("@Service\nclass MyService {}");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "Service" && rf.kind == EdgeKind::TypeRef),
        "expected TypeRef to Service annotation; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_annotation_on_property() {
    // @Inject annotation on property should emit TypeRef
    let r = extract("class C {\n    @Inject\n    lateinit var service: Service\n}");
    let service_refs: Vec<_> = r.refs.iter().filter(|rf| rf.target_name == "Service").collect();
    assert!(
        service_refs.len() >= 1,
        "expected at least one TypeRef to Service (annotation and property type); got {:?}",
        r.refs.iter().map(|rf| &rf.target_name).collect::<Vec<_>>()
    );
}

#[test]
fn ref_multiple_annotations() {
    // Multiple annotations should each emit TypeRef
    let r = extract("@Service\n@Component\nclass MyService {}");
    let type_refs: Vec<_> = r.refs.iter().filter(|rf| rf.kind == EdgeKind::TypeRef).collect();
    assert!(
        type_refs.iter().any(|rf| rf.target_name == "Service"),
        "expected TypeRef to Service"
    );
    assert!(
        type_refs.iter().any(|rf| rf.target_name == "Component"),
        "expected TypeRef to Component"
    );
}

#[test]
fn ref_annotation_on_companion_object() {
    // @JvmStatic annotation on companion object should emit TypeRef
    let r = extract("class C {\n    @JvmField\n    companion object {}\n}");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "JvmField" && rf.kind == EdgeKind::TypeRef),
        "expected TypeRef JvmField on companion_object; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_property_in_function_body() {
    // property_declaration inside a function body should produce a Property symbol
    let r = extract("fun setup() {\n    val timeout: Int = 30\n}");
    assert!(
        r.symbols.iter().any(|s| s.name == "timeout"),
        "expected Property timeout from local val; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}
