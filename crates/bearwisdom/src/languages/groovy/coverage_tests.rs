// =============================================================================
// groovy/coverage_tests.rs  —  One test per declared symbol_node_kind and ref_node_kind
//
// Grammar node kinds (confirmed by CST probe):
//   class_declaration  — class body
//   method_declaration — typed method inside class body
//   function_definition — top-level `def fn(...)`
//   package_declaration — package statement
//   import_declaration  — import statement
//   method_invocation   — call expression
// =============================================================================

use super::extract::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// Symbol node kinds
// ---------------------------------------------------------------------------

/// symbol_node_kind: `class_declaration`  →  Class
#[test]
fn symbol_class_definition() {
    let r = extract("class Foo {\n    def bar() { baz() }\n}");
    assert!(
        r.symbols.iter().any(|s| s.name == "Foo" && s.kind == SymbolKind::Class),
        "expected Class Foo; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `function_definition` (top-level `def`)  →  Function
#[test]
fn symbol_function_definition_top_level() {
    let r = extract("def greet(name) {\n    println(name)\n}");
    assert!(
        r.symbols.iter().any(|s| s.name == "greet" && s.kind == SymbolKind::Function),
        "expected Function greet; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `method_declaration` inside class  →  Method
#[test]
fn symbol_function_definition_method() {
    let r = extract("class Foo {\n    def bar() { baz() }\n}");
    assert!(
        r.symbols.iter().any(|s| s.name == "bar" && s.kind == SymbolKind::Method),
        "expected Method bar; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: typed `method_declaration`  →  Method
#[test]
fn symbol_function_declaration() {
    let r = extract("class Calc {\n    int add(int a, int b) { return a + b }\n}");
    assert!(
        r.symbols.iter().any(|s| s.name == "add" && s.kind == SymbolKind::Method),
        "expected Method add; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `package_declaration`  →  Namespace
#[test]
fn symbol_groovy_package() {
    let r = extract("package com.example.app\n\nclass Hello {}");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Namespace),
        "expected Namespace from package_declaration; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Ref node kinds
// ---------------------------------------------------------------------------

/// ref_node_kind: `method_invocation`  →  Calls edge
#[test]
fn ref_function_call() {
    let r = extract("class Foo {\n    def bar() { baz() }\n}");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "baz" && rf.kind == EdgeKind::Calls),
        "expected Calls baz; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// ref_node_kind: top-level `method_invocation` (like println)
#[test]
fn ref_juxt_function_call() {
    let r = extract("def run() {\n    println(\"hello\")\n}");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "println" && rf.kind == EdgeKind::Calls),
        "expected Calls println; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// ref_node_kind: `import_declaration`  →  Imports edge
#[test]
fn ref_groovy_import() {
    let r = extract("import groovy.json.JsonSlurper\n\nclass Foo {}");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports),
        "expected Imports from import_declaration; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Additional symbol node kinds — declaration (Field / Variable), nested class,
// package name format
// ---------------------------------------------------------------------------

/// `declaration` inside a class body → Field symbol
// TODO: extractor does not extract `declaration` nodes as Field symbols.
// #[test]
// fn symbol_field_declaration() {
//     let r = extract("class Foo {\n    String name = \"hello\"\n}");
//     assert!(
//         r.symbols.iter().any(|s| s.name == "name" && s.kind == SymbolKind::Field),
//         "expected Field name; got {:?}",
//         r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
//     );
// }

/// `package_declaration` → Namespace with fully qualified dotted name
#[test]
fn symbol_groovy_package_name_format() {
    let r = extract("package com.example.app\n\nclass Hello {}");
    let ns: Vec<_> = r
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Namespace)
        .collect();
    assert!(
        !ns.is_empty(),
        "expected Namespace symbol from package; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    assert!(
        ns[0].name.contains('.'),
        "Namespace name should be dotted (com.example.app); got '{}'",
        ns[0].name
    );
}

/// Nested class → Class symbol with parent_index pointing to outer class
#[test]
fn symbol_nested_class() {
    let r = extract("class Outer {\n    class Inner {}\n}");
    assert!(
        r.symbols.iter().any(|s| s.name == "Outer" && s.kind == SymbolKind::Class),
        "expected Class Outer; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.symbols.iter().any(|s| s.name == "Inner" && s.kind == SymbolKind::Class),
        "expected Class Inner; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    let inner = r.symbols.iter().find(|s| s.name == "Inner").unwrap();
    assert!(
        inner.parent_index.is_some(),
        "nested class Inner should have parent_index set; got {:?}",
        inner.parent_index
    );
}

// ---------------------------------------------------------------------------
// Additional ref node kinds — wildcard import, chained method call
// ---------------------------------------------------------------------------

/// Wildcard import → Imports edge (module name without trailing `.*`)
#[test]
fn ref_groovy_wildcard_import() {
    let r = extract("import groovy.json.*\n\nclass Foo {}");
    assert!(
        r.refs
            .iter()
            .any(|rf| rf.kind == EdgeKind::Imports && rf.target_name.contains("groovy.json")),
        "expected Imports from wildcard import; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// Multiple method calls in a method body → multiple Calls edges
#[test]
fn ref_multiple_calls_in_method() {
    let r = extract("class Foo {\n    def run() {\n        bar()\n        baz()\n    }\n}");
    let calls: Vec<_> = r.refs.iter().filter(|rf| rf.kind == EdgeKind::Calls).collect();
    assert!(
        calls.len() >= 2,
        "expected >= 2 Calls edges; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// `class extends Super` → Inherits ref
// TODO: extractor does not emit Inherits edges from class_declaration.
// #[test]
// fn ref_class_extends_produces_inherits() {
//     let r = extract("class Dog extends Animal {}");
//     assert!(
//         r.refs.iter().any(|rf| rf.kind == EdgeKind::Inherits && rf.target_name == "Animal"),
//         "expected Inherits(Animal) from extends clause; got {:?}",
//         r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
//     );
// }

/// `class implements Interface` → Implements ref
// TODO: extractor does not emit Implements edges from class_declaration.
// #[test]
// fn ref_class_implements_produces_implements() {
//     let r = extract("class Foo implements IBar {}");
//     assert!(
//         r.refs.iter().any(|rf| rf.kind == EdgeKind::Implements && rf.target_name == "IBar"),
//         "expected Implements(IBar) from implements clause; got {:?}",
//         r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
//     );
// }

/// Method with typed return type → Method symbol still emitted
#[test]
fn symbol_method_with_return_type_produces_method() {
    let r = extract("class Service {\n    List<String> getNames() {\n        return []\n    }\n}");
    assert!(
        r.symbols.iter().any(|s| s.name == "getNames" && s.kind == SymbolKind::Method),
        "expected Method getNames; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}
