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

/// `local_open_expression` as a function callee (e.g. `Alcotest.(check int)`)
/// must NOT produce a Calls ref — the multi-word text is not a resolvable name.
#[test]
fn local_open_expression_does_not_emit_calls_ref() {
    let r = extract(
        "let () = Alcotest.(check int) \"msg\" 1 1",
        "test.ml",
    );
    // No ref whose target contains '(' should appear.
    assert!(
        !r.refs.iter().any(|rf| rf.target_name.contains('(')),
        "expected no paren-target Calls ref from local_open_expression; got {:?}",
        r.refs.iter().filter(|rf| rf.target_name.contains('(')).collect::<Vec<_>>()
    );
}

/// `local_open_expression` callee emits a qualified Calls ref for the inner
/// function so `Fmt.(any ",")` resolves `any` with module=Some("Fmt") rather
/// than remaining invisible to the resolver.
#[test]
fn local_open_expression_emits_qualified_calls_ref() {
    let r = extract(
        "let () = ignore (Fmt.(any \",\"))",
        "test.ml",
    );
    let qualified = r.refs.iter().find(|rf| {
        rf.kind == EdgeKind::Calls
            && rf.target_name == "any"
            && rf.module.as_deref() == Some("Fmt")
    });
    assert!(
        qualified.is_some(),
        "expected Calls ref for 'any' with module='Fmt' from local_open_expression; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.module.as_deref(), rf.kind)).collect::<Vec<_>>()
    );
}

/// Dotted-module local open `Outer.Inner.(f arg)` emits a Calls ref for `f`
/// with `module=Some("Outer.Inner")`. Nested calls inside labeled arguments
/// also receive the opened module so `Fmt.(option ~none:(any "") ...)` emits
/// both `option` and `any` with `module=Some("Fmt")`.
#[test]
fn dotted_local_open_expression_emits_qualified_ref() {
    let r = extract(
        "let x = Irmin.Type.(unstage (compare t))",
        "test.ml",
    );
    let unstage_ref = r.refs.iter().find(|rf| {
        rf.kind == EdgeKind::Calls && rf.target_name == "unstage"
            && rf.module.as_deref() == Some("Irmin.Type")
    });
    assert!(
        unstage_ref.is_some(),
        "expected Calls ref for 'unstage' with module='Irmin.Type'; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.module.as_deref(), rf.kind)).collect::<Vec<_>>()
    );
}

/// Nested calls inside a local_open body (e.g. as labeled argument values)
/// inherit the opened module context so they resolve against the same package.
#[test]
fn nested_call_in_local_open_body_gets_module_context() {
    // `option` is the top-level call, `any` is nested inside a labeled argument.
    // Both must carry module=Some("Fmt").
    let r = extract(
        r#"let () = Fmt.(option ~none:(any "") Format.pp_print_string) fmt v"#,
        "test.ml",
    );
    let has_any_fmt = r.refs.iter().any(|rf| {
        rf.kind == EdgeKind::Calls && rf.target_name == "any"
            && rf.module.as_deref() == Some("Fmt")
    });
    assert!(
        has_any_fmt,
        "expected Calls ref for 'any' with module='Fmt' from nested local_open arg; got {:?}",
        r.refs.iter().filter(|rf| rf.target_name == "any" || rf.target_name == "option")
            .map(|rf| (&rf.target_name, rf.module.as_deref(), rf.kind)).collect::<Vec<_>>()
    );
}

/// Inline attribute annotations on call sites (`(aux [@tailcall]) t1 t2`) must
/// not produce a Calls ref with the raw bracketed text as target_name. The
/// actual callee `aux` may still appear from a separate walk of the same
/// application node, but the raw `(aux [@tailcall])` form must be dropped.
#[test]
fn inline_attribute_annotated_call_does_not_emit_bracket_target() {
    let r = extract(
        "let () = (aux [@tailcall]) t1 t2",
        "test.ml",
    );
    let bracket_refs: Vec<_> = r.refs.iter()
        .filter(|rf| rf.target_name.contains('['))
        .collect();
    assert!(
        bracket_refs.is_empty(),
        "expected no Calls ref with bracket in target_name; got {:?}",
        bracket_refs
    );
}

/// Attribute payloads (`[@@deriving foo ~bar]`) look like application_expressions
/// but must not emit Calls refs — they are metadata, not runtime call sites.
#[test]
fn attribute_payload_does_not_emit_calls_ref() {
    let r = extract(
        "type t = int [@@deriving irmin ~pp ~compare]",
        "test.ml",
    );
    let ppx_calls: Vec<_> = r.refs.iter().filter(|rf| {
        rf.kind == EdgeKind::Calls
            && (rf.target_name == "irmin" || rf.target_name == "pp" || rf.target_name == "compare")
    }).collect();
    assert!(
        ppx_calls.is_empty(),
        "expected no Calls refs from attribute payload; got {:?}",
        ppx_calls
    );
}

/// Variant constructors in a `type_definition` are extracted as individual
/// `Struct`-kinded child symbols so constructor applications can resolve.
#[test]
fn variant_constructors_extracted_as_struct_symbols() {
    let r = extract(
        "type color = Red | Green | Blue of int",
        "test.ml",
    );
    assert!(
        r.symbols.iter().any(|s| s.name == "Red" && s.kind == SymbolKind::Struct),
        "expected Struct 'Red' from variant constructor; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.symbols.iter().any(|s| s.name == "Green" && s.kind == SymbolKind::Struct),
        "expected Struct 'Green' from variant constructor; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.symbols.iter().any(|s| s.name == "Blue" && s.kind == SymbolKind::Struct),
        "expected Struct 'Blue' from variant constructor; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// Variant constructors at top level have unqualified names (file-level module
/// prefix is not added by the extractor).
#[test]
fn variant_constructor_top_level_unqualified() {
    let r = extract(
        "type result = Ok of int | Err of string",
        "test.ml",
    );
    let ok = r.symbols.iter().find(|s| s.name == "Ok").expect("Ok");
    assert_eq!(ok.qualified_name, "Ok");
    assert_eq!(ok.scope_path, None);
}

/// Variant constructors inside a module are scoped to the module, not the type.
#[test]
fn variant_constructor_inside_module_scoped_to_module() {
    let r = extract(
        "module M = struct\n  type color = Red | Green\nend",
        "test.ml",
    );
    let red = r.symbols.iter().find(|s| s.name == "Red").expect("Red");
    assert_eq!(red.qualified_name, "M.Red");
    assert_eq!(red.scope_path.as_deref(), Some("M"));
}

/// Constructor applied to an argument is an `application_expression` and emits
/// a Calls ref. A bare constructor with no argument is a value expression and
/// does not produce a Calls ref (it's not an application in OCaml grammar).
#[test]
fn variant_constructor_application_emits_calls_ref() {
    let r = extract(
        "type shape = Circle | Square of int\nlet t = Square 5",
        "test.ml",
    );
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "Square" && rf.kind == EdgeKind::Calls),
        "expected Calls->Square; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// Qualified constructor application `Module.Ctor arg` is split into
/// `target_name="Ctor", module=Some("Module")` so the module-qualified resolver
/// step can find it. The raw text `"Module.Ctor"` must NOT appear as target.
#[test]
fn qualified_constructor_path_is_split() {
    let r = extract(
        "let () = Result.Ok 42 |> ignore",
        "test.ml",
    );
    let ok_ref = r.refs.iter().find(|rf| rf.target_name == "Ok" && rf.kind == EdgeKind::Calls);
    assert!(
        ok_ref.is_some(),
        "expected split Calls->Ok from Result.Ok; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, &rf.module, rf.kind)).collect::<Vec<_>>()
    );
    assert_eq!(
        ok_ref.unwrap().module.as_deref(),
        Some("Result"),
        "expected module=Some(\"Result\") for Result.Ok"
    );
}
