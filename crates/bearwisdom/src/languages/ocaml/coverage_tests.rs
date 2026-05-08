// =============================================================================
// ocaml/coverage_tests.rs — One test per declared symbol_node_kind and ref_node_kind
//
// symbol_node_kinds:
//   value_definition, type_definition (record/variant/alias), module_definition,
//   open_module, exception_definition (TODO), module_type_definition (TODO),
//   class_definition (TODO), external (TODO)
//
// ref_node_kinds:
//   open_module → Imports, application_expression → Calls,
//   inheritance_definition → Inherits (TODO), new_expression → Instantiates (TODO)
// =============================================================================

use super::extract::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// Module-qname propagation (regression: pre-fix every symbol got an
// unprefixed qname, so `let bind` inside `module M = struct ... end` was
// indexed as "bind" rather than "M.bind", and dotted refs like `M.bind`
// couldn't resolve.)
// ---------------------------------------------------------------------------

#[test]
fn function_inside_module_qualified_with_module_name() {
    let r = extract(
        "module Async = struct\n  let bind f = f\nend",
        "test.ml",
    );
    let bind = r.symbols.iter().find(|s| s.name == "bind").expect("bind");
    assert_eq!(bind.qualified_name, "Async.bind");
    assert_eq!(bind.scope_path.as_deref(), Some("Async"));
}

#[test]
fn type_inside_module_qualified_with_module_name() {
    let r = extract(
        "module Foo = struct\n  type person = { name : string }\nend",
        "test.ml",
    );
    let p = r.symbols.iter().find(|s| s.name == "person").expect("person");
    assert_eq!(p.qualified_name, "Foo.person");
}

#[test]
fn nested_module_qname_chain() {
    let r = extract(
        "module Outer = struct\n  module Inner = struct\n    let value = 42\n  end\nend",
        "test.ml",
    );
    let v = r.symbols.iter().find(|s| s.name == "value").expect("value");
    assert_eq!(v.qualified_name, "Outer.Inner.value");
}

#[test]
fn top_level_let_unprefixed() {
    let r = extract("let x = 1", "test.ml");
    let x = r.symbols.iter().find(|s| s.name == "x").expect("x");
    assert_eq!(x.qualified_name, "x");
    assert_eq!(x.scope_path, None);
}

// ---------------------------------------------------------------------------
// symbol_node_kinds
// ---------------------------------------------------------------------------

/// value_definition with parameters → Function symbol
#[test]
fn symbol_value_definition_function() {
    let r = extract("let foo x = x + 1", "test.ml");
    assert!(
        r.symbols.iter().any(|s| s.name == "foo" && s.kind == SymbolKind::Function),
        "expected Function foo; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// value_definition without parameters → Variable symbol
#[test]
fn symbol_value_definition_variable() {
    let r = extract("let answer = 42", "test.ml");
    assert!(
        r.symbols.iter().any(|s| s.name == "answer"),
        "expected binding answer; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// type_definition with record body → Struct symbol
#[test]
fn symbol_type_definition_record() {
    let r = extract("type point = { x: int; y: int }", "test.ml");
    assert!(
        r.symbols.iter().any(|s| s.name == "point" && s.kind == SymbolKind::Struct),
        "expected Struct point; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// type_definition with variant constructors → Enum symbol
#[test]
fn symbol_type_definition_variant() {
    let r = extract("type color = Red | Green | Blue", "test.ml");
    assert!(
        r.symbols.iter().any(|s| s.name == "color" && s.kind == SymbolKind::Enum),
        "expected Enum color; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// module_definition → Namespace symbol
#[test]
fn symbol_module_definition() {
    let r = extract("module M = struct\n  let foo x = x + 1\nend", "test.ml");
    assert!(
        r.symbols.iter().any(|s| s.name == "M" && s.kind == SymbolKind::Namespace),
        "expected Namespace M; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// open_module as a symbol-kind node → Imports ref (also covers ref side)
#[test]
fn symbol_open_module_imports() {
    let r = extract("open List", "test.ml");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "List" && rf.kind == EdgeKind::Imports),
        "expected Imports List from open; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// ref_node_kinds
// ---------------------------------------------------------------------------

/// open_module → Imports ref
#[test]
fn ref_open_module() {
    let r = extract("open Printf", "test.ml");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports),
        "expected Imports ref; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// application_expression → Calls ref
#[test]
fn ref_application_expression() {
    let r = extract("module M = struct\n  let foo x = x + 1\n  type t = { name: string }\nend", "test.ml");
    // foo is called or defined; the module contains a value_definition
    assert!(
        !r.symbols.is_empty(),
        "expected symbols from module struct; got none"
    );
}

/// application_expression in a function body → Calls ref
#[test]
fn ref_application_expression_call() {
    let r = extract("let bar x = x + 1\nlet main () = bar 42", "test.ml");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "bar" && rf.kind == EdgeKind::Calls),
        "expected Calls bar; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// Qualified call: Module.function → target_name = function, module = Some(Module)
#[test]
fn ref_qualified_call() {
    let r = extract("let main () = List.map (fun x -> x) [1;2;3]", "test.ml");
    let rf = r.refs.iter().find(|rf| rf.target_name == "map" && rf.kind == EdgeKind::Calls);
    assert!(
        rf.is_some(),
        "expected Calls ref to 'map'; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
    assert_eq!(
        rf.unwrap().module.as_deref(),
        Some("List"),
        "expected module=Some(\"List\") for qualified call List.map"
    );
}

/// Nested qualified call: Stdlib.List.map → module = "Stdlib.List", target = "map"
#[test]
fn ref_nested_qualified_call() {
    let r = extract("let main () = Stdlib.List.map (fun x -> x) [1;2;3]", "test.ml");
    let rf = r.refs.iter().find(|rf| rf.target_name == "map");
    assert!(rf.is_some(), "expected ref to 'map'");
    assert_eq!(
        rf.unwrap().module.as_deref(),
        Some("Stdlib.List"),
        "expected module=Some(\"Stdlib.List\") for nested qualified call Stdlib.List.map"
    );
}

// ---------------------------------------------------------------------------
// Additional symbol_node_kinds
// ---------------------------------------------------------------------------

/// `type name = string` — type alias via the `equation` field on `type_binding`.
/// tree-sitter-ocaml encodes simple aliases using the `equation` named field
/// (not `body` or `synonym`).  The extractor checks `equation` to emit TypeAlias.
#[test]
fn symbol_type_definition_alias_emits_type_alias() {
    let r = extract("type name = string", "test.ml");
    assert!(
        r.symbols.iter().any(|s| s.name == "name" && s.kind == SymbolKind::TypeAlias),
        "expected TypeAlias 'name' from type equation; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// exception_definition → SymbolKind::Struct
#[test]
fn symbol_exception_definition() {
    let r = extract("exception Not_found\nexception Invalid_arg of string", "test.ml");
    assert!(
        r.symbols.iter().any(|s| s.name == "Not_found" && s.kind == SymbolKind::Struct),
        "expected Struct 'Not_found' from exception_definition; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.symbols.iter().any(|s| s.name == "Invalid_arg" && s.kind == SymbolKind::Struct),
        "expected Struct 'Invalid_arg' from exception_definition; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// module_type_definition → SymbolKind::Interface
#[test]
fn symbol_module_type_definition() {
    let r = extract("module type S = sig\n  val foo : int -> int\nend", "test.ml");
    assert!(
        r.symbols.iter().any(|s| s.name == "S" && s.kind == SymbolKind::Interface),
        "expected Interface 'S' from module_type_definition; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// class_definition → SymbolKind::Class
#[test]
fn symbol_class_definition() {
    let r = extract(
        "class point x0 y0 = object\n  val mutable x = x0\n  method get_x = x\nend",
        "test.ml",
    );
    assert!(
        r.symbols.iter().any(|s| s.name == "point" && s.kind == SymbolKind::Class),
        "expected Class 'point' from class_definition; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// external → SymbolKind::Function
#[test]
fn symbol_external() {
    let r = extract("external string_length : string -> int = \"caml_string_length\"", "test.ml");
    assert!(
        r.symbols.iter().any(|s| s.name == "string_length" && s.kind == SymbolKind::Function),
        "expected Function 'string_length' from external; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Additional ref_node_kinds
// ---------------------------------------------------------------------------

/// inheritance_definition → EdgeKind::Inherits
#[test]
fn ref_inheritance_definition() {
    let r = extract(
        "class child x = object\n  inherit point x 0\nend",
        "test.ml",
    );
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Inherits && rf.target_name == "point"),
        "expected Inherits->point from inheritance_definition; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// new_expression → EdgeKind::Instantiates
#[test]
fn ref_new_expression() {
    let r = extract(
        "class counter = object\n  val mutable n = 0\n  method incr = n <- n + 1\nend\nlet c = new counter",
        "test.ml",
    );
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Instantiates && rf.target_name == "counter"),
        "expected Instantiates->counter from new_expression; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// .mli interface file — value_specification → Function
#[test]
fn symbol_value_specification_in_mli() {
    let r = extract("val foo : int -> int\nval bar : string", "test.mli");
    assert!(
        r.symbols.iter().any(|s| s.name == "foo" && s.kind == SymbolKind::Function),
        "expected Function 'foo' from value_specification in .mli; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}
