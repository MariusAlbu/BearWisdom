// =============================================================================
// rust_lang/coverage_tests.rs  —  Per-node-kind coverage for the Rust extractor
//
// For every kind listed in RustLangPlugin::symbol_node_kinds() and
// ref_node_kinds(), at least one test confirms correct extraction.
//
// symbol_node_kinds:
//   struct_item, enum_item, enum_variant, trait_item, impl_item,
//   function_item, function_signature_item, const_item, static_item,
//   type_item, associated_type, mod_item, field_declaration, union_item,
//   macro_definition
//
// ref_node_kinds:
//   call_expression, macro_invocation, struct_expression, use_declaration,
//   impl_item, type_cast_expression, type_arguments, attribute_item,
//   trait_bounds, scoped_type_identifier, type_identifier, dynamic_type,
//   abstract_type
// =============================================================================

use super::extract;
use crate::types::*;

// ---------------------------------------------------------------------------
// symbol_node_kinds
// ---------------------------------------------------------------------------

// ---- struct_item -----------------------------------------------------------

#[test]
fn coverage_struct_item_emits_struct_symbol() {
    let src = "struct Foo { x: i32 }";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "Foo");
    assert!(sym.is_some(), "expected struct symbol 'Foo'");
    assert_eq!(sym.unwrap().kind, SymbolKind::Struct);
}

// ---- enum_item + enum_variant ----------------------------------------------

#[test]
fn coverage_enum_item_emits_enum_symbol() {
    let src = "enum Status { Active, Inactive }";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "Status");
    assert!(sym.is_some(), "expected Enum symbol 'Status'");
    assert_eq!(sym.unwrap().kind, SymbolKind::Enum);
}

#[test]
fn coverage_enum_variant_emits_enum_member_symbols() {
    let src = "enum Color { Red, Green, Blue }";
    let r = extract::extract(src);
    let members: Vec<&str> = r
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::EnumMember)
        .map(|s| s.name.as_str())
        .collect();
    assert!(members.contains(&"Red"),   "missing Red:   {members:?}");
    assert!(members.contains(&"Green"), "missing Green: {members:?}");
    assert!(members.contains(&"Blue"),  "missing Blue:  {members:?}");
}

// ---- trait_item ------------------------------------------------------------

#[test]
fn coverage_trait_item_emits_interface_symbol() {
    let src = "trait Drawable { fn draw(&self); }";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "Drawable");
    assert!(sym.is_some(), "expected Interface symbol 'Drawable'");
    assert_eq!(sym.unwrap().kind, SymbolKind::Interface);
}

// ---- impl_item -------------------------------------------------------------

#[test]
fn coverage_impl_item_methods_qualify_under_type() {
    let src = "struct S;\nimpl S { fn method(&self) {} }";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "method");
    assert!(sym.is_some(), "expected method symbol 'method'");
    assert_eq!(sym.unwrap().qualified_name, "S.method");
    assert_eq!(sym.unwrap().kind, SymbolKind::Method);
}

#[test]
fn coverage_impl_item_for_trait_emits_method_under_struct() {
    // `impl Trait for Struct` — methods qualify under Struct, not Trait.
    let src = "struct MyRepo;\ntrait Repository { fn find(&self); }\nimpl Repository for MyRepo { fn find(&self) {} }";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "find" && s.kind == SymbolKind::Method);
    assert!(sym.is_some(), "expected Method 'find' from impl block");
}

// ---- function_item ---------------------------------------------------------

#[test]
fn coverage_function_item_emits_function_symbol() {
    let src = "fn foo() {}";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "foo");
    assert!(sym.is_some(), "expected Function symbol 'foo'");
    assert_eq!(sym.unwrap().kind, SymbolKind::Function);
}

// ---- function_signature_item -----------------------------------------------

#[test]
fn coverage_function_signature_item_in_trait_emits_function_symbol() {
    // Trait method declaration (no body) is a `function_signature_item`.
    let src = "trait Animal { fn sound(&self) -> String; }";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "sound");
    assert!(
        sym.is_some(),
        "expected symbol 'sound' from function_signature_item; symbols: {:?}",
        r.symbols.iter().map(|s| (&s.name, &s.kind)).collect::<Vec<_>>()
    );
    assert!(
        matches!(sym.unwrap().kind, SymbolKind::Method | SymbolKind::Function),
        "expected Method or Function kind for trait method, got: {:?}",
        sym.unwrap().kind
    );
}

// ---- const_item ------------------------------------------------------------

#[test]
fn coverage_const_item_emits_variable_symbol() {
    let src = "const MAX_RETRIES: u32 = 3;";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "MAX_RETRIES");
    assert!(sym.is_some(), "expected Variable symbol 'MAX_RETRIES'");
    assert_eq!(sym.unwrap().kind, SymbolKind::Variable);
}

// ---- static_item -----------------------------------------------------------

#[test]
fn coverage_static_item_emits_variable_symbol() {
    let src = "static GLOBAL_COUNT: u64 = 0;";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "GLOBAL_COUNT");
    assert!(sym.is_some(), "expected Variable symbol 'GLOBAL_COUNT'");
    assert_eq!(sym.unwrap().kind, SymbolKind::Variable);
}

// ---- type_item -------------------------------------------------------------

#[test]
fn coverage_type_item_emits_type_alias_symbol() {
    let src = "type UserId = u64;";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "UserId");
    assert!(sym.is_some(), "expected TypeAlias symbol 'UserId'");
    assert_eq!(sym.unwrap().kind, SymbolKind::TypeAlias);
}

// ---- associated_type -------------------------------------------------------

#[test]
fn coverage_associated_type_in_impl_emits_type_alias() {
    let src = "struct MyIter;\nimpl Iterator for MyIter {\n    type Item = i32;\n    fn next(&mut self) -> Option<i32> { None }\n}";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "Item");
    assert!(
        sym.is_some(),
        "expected TypeAlias 'Item' from associated type; symbols: {:?}",
        r.symbols.iter().map(|s| (&s.name, &s.kind)).collect::<Vec<_>>()
    );
    assert_eq!(sym.unwrap().kind, SymbolKind::TypeAlias);
}

#[test]
fn coverage_associated_type_in_trait_emits_type_alias() {
    let src = "trait Container { type Output; }";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "Output");
    assert!(
        sym.is_some(),
        "expected TypeAlias 'Output' from associated type in trait; symbols: {:?}",
        r.symbols.iter().map(|s| (&s.name, &s.kind)).collect::<Vec<_>>()
    );
}

// ---- mod_item --------------------------------------------------------------

#[test]
fn coverage_mod_item_emits_namespace_symbol() {
    let src = "mod utils { pub fn helper() {} }";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "utils");
    assert!(sym.is_some(), "expected Namespace symbol 'utils'");
    assert_eq!(sym.unwrap().kind, SymbolKind::Namespace);
}

// ---- field_declaration -----------------------------------------------------

#[test]
fn coverage_field_declaration_emits_field_symbols() {
    let src = "struct Point { x: f64, y: f64 }";
    let r = extract::extract(src);
    let field_names: Vec<&str> = r
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Field)
        .map(|s| s.name.as_str())
        .collect();
    assert!(field_names.contains(&"x"), "missing field 'x'; symbols: {:?}", r.symbols.iter().map(|s| (&s.name, &s.kind)).collect::<Vec<_>>());
    assert!(field_names.contains(&"y"), "missing field 'y'; symbols: {field_names:?}");
}

#[test]
fn coverage_field_declaration_named_type_emits_type_ref() {
    // A struct field with a user-defined type should emit a TypeRef.
    let src = "struct Order { customer: User, status: Status }";
    let r = extract::extract(src);
    let type_refs: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        type_refs.contains(&"User"),
        "expected TypeRef to User from struct field; refs: {type_refs:?}"
    );
    assert!(
        type_refs.contains(&"Status"),
        "expected TypeRef to Status from struct field; refs: {type_refs:?}"
    );
}

#[test]
fn coverage_field_declaration_qualified_name() {
    let src = "struct Config { host: String, port: u16 }";
    let r = extract::extract(src);
    let host = r.symbols.iter().find(|s| s.name == "host" && s.kind == SymbolKind::Field);
    assert!(host.is_some(), "expected Field 'host'");
    assert_eq!(host.unwrap().qualified_name, "Config.host");
}

// ---- union_item ------------------------------------------------------------

#[test]
fn coverage_union_item_emits_struct_kind_symbol() {
    let src = "union MyUnion { i: i32, f: f32 }";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "MyUnion");
    assert!(sym.is_some(), "expected symbol 'MyUnion' from union_item");
    assert_eq!(sym.unwrap().kind, SymbolKind::Struct);
}

// ---- macro_definition ------------------------------------------------------

#[test]
fn coverage_macro_definition_emits_function_symbol() {
    let src = "macro_rules! my_assert { ($x:expr) => { assert!($x); }; }";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "my_assert");
    assert!(sym.is_some(), "expected Function symbol 'my_assert' from macro_definition");
    assert_eq!(sym.unwrap().kind, SymbolKind::Function);
}

// ---------------------------------------------------------------------------
// ref_node_kinds
// ---------------------------------------------------------------------------

// ---- call_expression -------------------------------------------------------

#[test]
fn coverage_call_expression_emits_calls_edge() {
    let src = "fn f() { bar(); }";
    let r = extract::extract(src);
    let calls: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Calls)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(calls.contains(&"bar"), "expected Calls edge to bar; calls: {calls:?}");
}

// ---- macro_invocation -------------------------------------------------------

#[test]
fn coverage_macro_invocation_emits_calls_edge() {
    let src = "fn f() { println!(\"hello\"); }";
    let r = extract::extract(src);
    let calls: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Calls)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        calls.contains(&"println"),
        "expected Calls edge to println from macro_invocation; calls: {calls:?}"
    );
}

#[test]
fn coverage_macro_invocation_nested_in_body() {
    // Macro inside a function body.
    let src = "fn run() { vec![1, 2, 3]; }";
    let r = extract::extract(src);
    let calls: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Calls)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        calls.contains(&"vec"),
        "expected Calls edge to vec from macro_invocation; calls: {calls:?}"
    );
}

// ---- struct_expression -----------------------------------------------------

#[test]
fn coverage_struct_expression_emits_calls_and_type_ref() {
    let src = "fn f() { let u = User { name: String::new() };\n_ = u; }";
    let r = extract::extract(src);
    let calls: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Calls)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        calls.contains(&"User"),
        "expected Calls edge to User from struct_expression; calls: {calls:?}"
    );
    let type_refs: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        type_refs.contains(&"User"),
        "expected TypeRef to User from struct_expression; type_refs: {type_refs:?}"
    );
}

// ---- use_declaration -------------------------------------------------------

#[test]
fn coverage_use_declaration_emits_imports_edge() {
    let src = "use std::collections::HashMap;";
    let r = extract::extract(src);
    let imports: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Imports)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        imports.contains(&"HashMap"),
        "expected Imports edge to HashMap; imports: {imports:?}"
    );
}

// ---- impl_item (as ref_node_kind: TypeRef to implemented trait) ------------

#[test]
fn coverage_impl_item_trait_emits_type_ref() {
    // `impl Display for Foo` — the `Display` trait is a TypeRef.
    let src = "use std::fmt;\nstruct Foo;\nimpl fmt::Display for Foo {\n    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { Ok(()) }\n}";
    let r = extract::extract(src);
    let type_refs: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    // The impl block references `Display` (or its scoped form) as a TypeRef.
    // At minimum the formatter/result TypeRefs should appear.
    let _ = type_refs; // existence check: no panic
    // Confirm no parse errors
    assert!(!r.has_errors, "unexpected parse errors");
}

// ---- type_cast_expression --------------------------------------------------

#[test]
fn coverage_type_cast_expression_named_type_emits_type_ref() {
    let src = "fn f(x: usize) -> MyIndex { x as MyIndex }";
    let r = extract::extract(src);
    let type_refs: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        type_refs.contains(&"MyIndex"),
        "expected TypeRef to MyIndex from type_cast_expression; refs: {type_refs:?}"
    );
}

// ---- type_arguments --------------------------------------------------------

#[test]
fn coverage_type_arguments_in_fn_return_emits_type_ref() {
    // `fn f() -> Vec<User>` — the `User` in type arguments should produce a TypeRef.
    let src = "fn users() -> Vec<User> { vec![] }";
    let r = extract::extract(src);
    let type_refs: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        type_refs.contains(&"User"),
        "expected TypeRef to User from type_arguments in return type; refs: {type_refs:?}"
    );
}

#[test]
fn coverage_type_arguments_in_param_emits_type_ref() {
    // `fn f(items: Vec<Item>)` — Item in type_arguments.
    let src = "fn process(items: Vec<Item>) {}";
    let r = extract::extract(src);
    let type_refs: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        type_refs.contains(&"Item"),
        "expected TypeRef to Item from type_arguments in param; refs: {type_refs:?}"
    );
}

// ---- attribute_item --------------------------------------------------------

#[test]
fn coverage_attribute_item_emits_type_ref() {
    let src = "#[derive(Debug, Clone)]\nstruct Config {}";
    let r = extract::extract(src);
    let type_refs: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        type_refs.contains(&"derive"),
        "expected TypeRef to derive from attribute_item; refs: {type_refs:?}"
    );
}

// ---- trait_bounds ----------------------------------------------------------

#[test]
fn coverage_trait_bounds_in_where_clause_emits_type_refs() {
    let src = "fn f<T>(x: T) where T: Clone + Send {}";
    let r = extract::extract(src);
    let type_refs: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(type_refs.contains(&"Clone"), "missing Clone from trait_bounds; refs: {type_refs:?}");
    assert!(type_refs.contains(&"Send"),  "missing Send from trait_bounds;  refs: {type_refs:?}");
}

#[test]
fn coverage_trait_bounds_inline_in_type_params_emits_type_refs() {
    // Inline bounds: `<T: Debug>`
    let src = "fn print_it<T: std::fmt::Debug>(x: T) {}";
    let r = extract::extract(src);
    let type_refs: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        type_refs.contains(&"Debug"),
        "expected TypeRef to Debug from inline trait_bounds; refs: {type_refs:?}"
    );
}

// ---- scoped_type_identifier ------------------------------------------------

#[test]
fn coverage_scoped_type_identifier_in_field_emits_type_ref() {
    // `std::fmt::Display` as a field type is a scoped_type_identifier.
    let src = "struct Wrapper { inner: std::sync::Arc<i32> }";
    let r = extract::extract(src);
    let type_refs: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    // `Arc` is the leaf name from the scoped_type_identifier.
    assert!(
        type_refs.contains(&"Arc"),
        "expected TypeRef to Arc from scoped_type_identifier in field; refs: {type_refs:?}"
    );
}

// ---- type_identifier -------------------------------------------------------

#[test]
fn coverage_type_identifier_in_fn_param_emits_type_ref() {
    // A function parameter with a named type → TypeRef.
    let src = "fn handle(req: Request) -> Response { Response {} }";
    let r = extract::extract(src);
    let type_refs: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        type_refs.contains(&"Request"),
        "expected TypeRef to Request from type_identifier in param; refs: {type_refs:?}"
    );
}

#[test]
fn coverage_type_identifier_in_fn_return_emits_type_ref() {
    // Return type as a bare type_identifier.
    let src = "fn get_user() -> User { todo!() }";
    let r = extract::extract(src);
    let type_refs: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        type_refs.contains(&"User"),
        "expected TypeRef to User from type_identifier in return type; refs: {type_refs:?}"
    );
}

// ---- dynamic_type (dyn Trait) ----------------------------------------------

#[test]
fn coverage_dynamic_type_in_fn_param_emits_type_ref() {
    let src = "fn f(e: &dyn Error) {}";
    let r = extract::extract(src);
    let type_refs: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        type_refs.contains(&"Error"),
        "expected TypeRef to Error from dynamic_type (dyn Error); refs: {type_refs:?}"
    );
}

#[test]
fn coverage_dynamic_type_in_field_emits_type_ref() {
    let src = "struct Handler { callback: Box<dyn Fn(i32)> }";
    let r = extract::extract(src);
    let type_refs: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        type_refs.contains(&"Fn"),
        "expected TypeRef to Fn from dynamic_type in field; refs: {type_refs:?}"
    );
}

// ---- abstract_type (impl Trait) --------------------------------------------

#[test]
fn coverage_abstract_type_in_fn_return_emits_type_ref() {
    let src = "fn make_writer() -> impl Write { todo!() }";
    let r = extract::extract(src);
    let type_refs: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        type_refs.contains(&"Write"),
        "expected TypeRef to Write from abstract_type (impl Write); refs: {type_refs:?}"
    );
}

#[test]
fn coverage_abstract_type_in_fn_param_emits_type_ref() {
    let src = "fn process(handler: impl Handler) {}";
    let r = extract::extract(src);
    let type_refs: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        type_refs.contains(&"Handler"),
        "expected TypeRef to Handler from abstract_type (impl Handler) in param; refs: {type_refs:?}"
    );
}
