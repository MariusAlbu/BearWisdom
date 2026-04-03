// =============================================================================
// go/coverage_tests.rs  —  Per-node-kind coverage tests for the Go extractor
//
// For every kind listed in GoPlugin::symbol_node_kinds() and ref_node_kinds(),
// at least one test confirms the extractor handles it correctly.
//
// symbol_node_kinds:
//   function_declaration, method_declaration, type_spec, type_alias,
//   const_spec, var_spec, field_declaration, method_elem, package_clause
//
// ref_node_kinds:
//   call_expression, import_spec, composite_literal,
//   type_conversion_expression, type_assertion_expression, selector_expression,
//   qualified_type, type_identifier
// =============================================================================

use super::extract;
use crate::types::*;

// ---------------------------------------------------------------------------
// symbol_node_kinds
// ---------------------------------------------------------------------------

// ---- function_declaration --------------------------------------------------

#[test]
fn coverage_function_declaration_emits_function_symbol() {
    let src = "package main\nfunc foo() {}";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "foo");
    assert!(sym.is_some(), "expected function symbol 'foo'");
    assert_eq!(sym.unwrap().kind, SymbolKind::Function);
}

// ---- method_declaration ----------------------------------------------------

#[test]
fn coverage_method_declaration_emits_method_symbol() {
    let src = "package main\ntype S struct{}\nfunc (s S) Method() {}";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "Method");
    assert!(sym.is_some(), "expected method symbol 'Method'");
    assert_eq!(sym.unwrap().kind, SymbolKind::Method);
    assert_eq!(sym.unwrap().qualified_name, "main.S.Method");
}

// ---- type_spec (struct) ----------------------------------------------------

#[test]
fn coverage_type_spec_struct_emits_struct_symbol() {
    let src = "package main\ntype User struct { Name string }";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "User");
    assert!(sym.is_some(), "expected struct symbol 'User'");
    assert_eq!(sym.unwrap().kind, SymbolKind::Struct);
}

// ---- type_spec (interface) -------------------------------------------------

#[test]
fn coverage_type_spec_interface_emits_interface_symbol() {
    let src = "package main\ntype Writer interface { Write(p []byte) (int, error) }";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "Writer");
    assert!(sym.is_some(), "expected interface symbol 'Writer'");
    assert_eq!(sym.unwrap().kind, SymbolKind::Interface);
}

// ---- type_alias ------------------------------------------------------------

#[test]
fn coverage_type_alias_emits_type_alias_symbol() {
    // `type Foo = Bar` uses the `type_alias` node in tree-sitter-go.
    let src = "package main\ntype MyStr = string";
    let r = extract::extract(src);
    // May be type_alias or type_spec depending on grammar version.
    // Either way the symbol should be emitted.
    let sym = r.symbols.iter().find(|s| s.name == "MyStr");
    assert!(sym.is_some(), "expected TypeAlias symbol 'MyStr'; got: {:?}", r.symbols.iter().map(|s| &s.name).collect::<Vec<_>>());
    assert_eq!(sym.unwrap().kind, SymbolKind::TypeAlias);
}

// ---- const_spec ------------------------------------------------------------

#[test]
fn coverage_const_spec_emits_variable_symbol() {
    let src = "package main\nconst MaxSize = 100";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "MaxSize");
    assert!(sym.is_some(), "expected Variable symbol for 'MaxSize'");
    assert_eq!(sym.unwrap().kind, SymbolKind::Variable);
}

// ---- var_spec --------------------------------------------------------------

#[test]
fn coverage_var_spec_emits_variable_symbol() {
    let src = "package main\nvar DefaultName string";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "DefaultName");
    assert!(sym.is_some(), "expected Variable symbol for 'DefaultName'");
    assert_eq!(sym.unwrap().kind, SymbolKind::Variable);
}

// ---- field_declaration -----------------------------------------------------

#[test]
fn coverage_field_declaration_emits_field_symbol() {
    let src = "package main\ntype User struct { Name string\nAge int }";
    let r = extract::extract(src);
    let name_field = r.symbols.iter().find(|s| s.name == "Name" && s.kind == SymbolKind::Field);
    assert!(name_field.is_some(), "expected Field symbol 'Name'; symbols: {:?}", r.symbols.iter().map(|s| (&s.name, &s.kind)).collect::<Vec<_>>());
    let age_field = r.symbols.iter().find(|s| s.name == "Age" && s.kind == SymbolKind::Field);
    assert!(age_field.is_some(), "expected Field symbol 'Age'");
}

// ---- field_declaration: named type → TypeRef -------------------------------

#[test]
fn coverage_field_declaration_named_type_emits_type_ref() {
    // A struct field with a user-defined type should emit a TypeRef.
    let src = "package main\ntype Order struct { Customer User }";
    let r = extract::extract(src);
    let type_refs: Vec<_> = r.refs.iter().filter(|r| r.kind == EdgeKind::TypeRef).collect();
    assert!(
        type_refs.iter().any(|r| r.target_name == "User"),
        "expected TypeRef to User from struct field; refs: {:?}",
        type_refs.iter().map(|r| &r.target_name).collect::<Vec<_>>()
    );
}

// ---- method_elem -----------------------------------------------------------

#[test]
fn coverage_method_elem_in_interface_emits_method_symbol() {
    let src = "package main\ntype Repo interface { Find(id int) User }";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "Find" && s.kind == SymbolKind::Method);
    assert!(sym.is_some(), "expected Method symbol 'Find'; symbols: {:?}", r.symbols.iter().map(|s| (&s.name, &s.kind)).collect::<Vec<_>>());
}

// ---- package_clause --------------------------------------------------------

#[test]
fn coverage_package_clause_qualifies_symbols() {
    // The package name from `package_clause` should appear in qualified_name.
    let src = "package mypkg\nfunc Run() {}";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "Run").expect("no Run");
    assert_eq!(sym.qualified_name, "mypkg.Run");
}

#[test]
fn coverage_package_clause_emits_namespace_symbol() {
    // The `package_clause` node must produce a Namespace symbol so the
    // coverage system can match it via line-number correlation.
    let src = "package mypkg\nfunc Run() {}";
    let r = extract::extract(src);
    let ns = r
        .symbols
        .iter()
        .find(|s| s.name == "mypkg" && s.kind == SymbolKind::Namespace);
    assert!(
        ns.is_some(),
        "expected Namespace symbol 'mypkg' from package_clause; symbols: {:?}",
        r.symbols.iter().map(|s| (&s.name, &s.kind)).collect::<Vec<_>>()
    );
    // Must be on the same line as the package_clause node (line 0).
    assert_eq!(ns.unwrap().start_line, 0);
}

// ---------------------------------------------------------------------------
// ref_node_kinds
// ---------------------------------------------------------------------------

// ---- call_expression -------------------------------------------------------

#[test]
fn coverage_call_expression_emits_calls_edge() {
    let src = "package main\nfunc f() { fmt.Println() }";
    let r = extract::extract(src);
    let calls: Vec<_> = r.refs.iter().filter(|r| r.kind == EdgeKind::Calls).collect();
    assert!(
        calls.iter().any(|r| r.target_name == "Println"),
        "expected Calls edge to Println; calls: {:?}",
        calls.iter().map(|r| &r.target_name).collect::<Vec<_>>()
    );
}

// ---- import_spec -----------------------------------------------------------

#[test]
fn coverage_import_spec_emits_imports_edge() {
    let src = "package main\nimport \"fmt\"";
    let r = extract::extract(src);
    let imports: Vec<_> = r.refs.iter().filter(|r| r.kind == EdgeKind::Imports).collect();
    assert!(
        imports.iter().any(|r| r.target_name == "fmt"),
        "expected Imports edge to fmt; refs: {:?}",
        imports.iter().map(|r| &r.target_name).collect::<Vec<_>>()
    );
}

// ---- composite_literal -----------------------------------------------------

#[test]
fn coverage_composite_literal_emits_instantiates_edge() {
    let src = "package main\nfunc f() { u := User{Name: \"x\"}\n_ = u }";
    let r = extract::extract(src);
    let inst: Vec<_> = r.refs.iter().filter(|r| r.kind == EdgeKind::Instantiates).collect();
    assert!(
        inst.iter().any(|r| r.target_name == "User"),
        "expected Instantiates edge to User; refs: {:?}",
        inst.iter().map(|r| &r.target_name).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_composite_literal_qualified_type() {
    // `pkg.Type{...}` — qualified_type as the literal type.
    let src = "package main\nfunc f() { r := http.Request{Method: \"GET\"}\n_ = r }";
    let r = extract::extract(src);
    let inst: Vec<_> = r.refs.iter().filter(|r| r.kind == EdgeKind::Instantiates).collect();
    assert!(
        inst.iter().any(|r| r.target_name == "Request"),
        "expected Instantiates edge to Request from pkg.Type literal; refs: {:?}",
        inst.iter().map(|r| &r.target_name).collect::<Vec<_>>()
    );
}

// ---- type_conversion_expression --------------------------------------------

#[test]
fn coverage_type_conversion_expression_emits_type_ref() {
    let src = "package main\nfunc f(b Buffer) MyString { return MyString(b) }";
    let r = extract::extract(src);
    let type_refs: Vec<_> = r.refs.iter().filter(|r| r.kind == EdgeKind::TypeRef).collect();
    assert!(
        type_refs.iter().any(|r| r.target_name == "MyString"),
        "expected TypeRef to MyString from type conversion; refs: {:?}",
        type_refs.iter().map(|r| &r.target_name).collect::<Vec<_>>()
    );
}

// ---- type_assertion_expression ---------------------------------------------

#[test]
fn coverage_type_assertion_expression_emits_type_ref() {
    let src = "package main\nfunc f(x interface{}) {\n    if a, ok := x.(*Admin); ok { _ = a } }";
    let r = extract::extract(src);
    let type_refs: Vec<_> = r.refs.iter().filter(|r| r.kind == EdgeKind::TypeRef).collect();
    assert!(
        type_refs.iter().any(|r| r.target_name == "Admin"),
        "expected TypeRef to Admin from type assertion; refs: {:?}",
        type_refs.iter().map(|r| &r.target_name).collect::<Vec<_>>()
    );
}

// ---- selector_expression ---------------------------------------------------

#[test]
fn coverage_selector_expression_emits_calls_edge_with_chain() {
    // `repo.FindOne()` — selector_expression as the function of a call_expression.
    let src = "package main\nfunc f(repo Repo) {\n    user := repo.FindOne(1)\n    _ = user\n}";
    let r = extract::extract(src);
    let calls: Vec<_> = r.refs.iter().filter(|r| r.kind == EdgeKind::Calls).collect();
    assert!(
        calls.iter().any(|r| r.target_name == "FindOne"),
        "expected Calls edge to FindOne from selector_expression; calls: {:?}",
        calls.iter().map(|r| &r.target_name).collect::<Vec<_>>()
    );
    // The call should carry a chain [repo, FindOne].
    let call_ref = calls.iter().find(|r| r.target_name == "FindOne").unwrap();
    assert!(
        call_ref.chain.is_some(),
        "expected chain on FindOne call; got None"
    );
}

// ---- qualified_type --------------------------------------------------------

#[test]
fn coverage_qualified_type_in_composite_literal_emits_instantiates() {
    // `http.Request{...}` uses a qualified_type node.
    let src = "package main\nfunc f() { req := http.Request{Method: \"GET\"}\n_ = req }";
    let r = extract::extract(src);
    let inst: Vec<_> = r.refs.iter().filter(|r| r.kind == EdgeKind::Instantiates).collect();
    assert!(
        inst.iter().any(|r| r.target_name == "Request"),
        "expected Instantiates from qualified_type literal; refs: {:?}",
        inst.iter().map(|r| &r.target_name).collect::<Vec<_>>()
    );
}

// ---- type_identifier → TypeRef from struct field ---------------------------

#[test]
fn coverage_type_identifier_in_struct_field_emits_type_ref() {
    // A struct field with a user-defined named type uses type_identifier.
    // The extractor should emit a TypeRef for the type.
    let src = "package main\ntype Order struct { Manager Employee }";
    let r = extract::extract(src);
    let type_refs: Vec<_> = r.refs.iter().filter(|r| r.kind == EdgeKind::TypeRef).collect();
    assert!(
        type_refs.iter().any(|r| r.target_name == "Employee"),
        "expected TypeRef to Employee from struct field type_identifier; refs: {:?}",
        type_refs.iter().map(|r| &r.target_name).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_type_identifier_in_function_param_emits_type_ref() {
    // Function parameters with named types → TypeRef via extract_go_typed_params_as_symbols.
    let src = "package main\nfunc Handle(req Request) Response { return Response{} }";
    let r = extract::extract(src);
    let type_refs: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        type_refs.contains(&"Request"),
        "expected TypeRef to Request from param type_identifier; refs: {type_refs:?}"
    );
}

// ---- selector_expression (standalone, not in call) -------------------------

#[test]
fn coverage_selector_expression_standalone_emits_calls_edge() {
    // `pkg.Var` used as a value expression (not called) should emit a Calls edge.
    let src = "package main\nfunc f() { _ = http.StatusOK }";
    let r = extract::extract(src);
    let calls: Vec<_> = r.refs.iter().filter(|r| r.kind == EdgeKind::Calls).collect();
    assert!(
        calls.iter().any(|r| r.target_name == "StatusOK"),
        "expected Calls edge to StatusOK from standalone selector_expression; calls: {:?}",
        calls.iter().map(|r| &r.target_name).collect::<Vec<_>>()
    );
}

// ---- qualified_type in body → TypeRef --------------------------------------

#[test]
fn coverage_qualified_type_in_body_emits_type_ref() {
    // `var x pkg.Type` inside a function — qualified_type should emit a TypeRef.
    let src = "package main\nfunc f() { var req http.Request\n_ = req }";
    let r = extract::extract(src);
    let type_refs: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        type_refs.contains(&"Request"),
        "expected TypeRef to Request from qualified_type in body; refs: {type_refs:?}"
    );
}

// ---- type_identifier in body → TypeRef -------------------------------------

#[test]
fn coverage_type_identifier_in_body_var_decl_emits_type_ref() {
    // `var x User` inside a function body should emit a TypeRef to User.
    let src = "package main\nfunc f() { var u User\n_ = u }";
    let r = extract::extract(src);
    let type_refs: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        type_refs.contains(&"User"),
        "expected TypeRef to User from type_identifier in body var; refs: {type_refs:?}"
    );
}

// ---- var_declaration inside function body → Variable symbol ----------------

#[test]
fn coverage_var_decl_in_function_body_emits_variable_symbol() {
    // `var x int` inside a function body should emit a Variable symbol.
    let src = "package main\nfunc f() { var count int\n_ = count }";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "count" && s.kind == SymbolKind::Variable);
    assert!(
        sym.is_some(),
        "expected Variable symbol 'count' from var_declaration in body; symbols: {:?}",
        r.symbols.iter().map(|s| (&s.name, &s.kind)).collect::<Vec<_>>()
    );
}

// ---- var_spec with user-defined type → TypeRef ----------------------------

#[test]
fn coverage_var_spec_named_type_emits_type_ref() {
    // `var DefaultHandler Handler` at package level should emit a TypeRef to Handler.
    let src = "package main\nvar DefaultHandler Handler";
    let r = extract::extract(src);
    let type_refs: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        type_refs.contains(&"Handler"),
        "expected TypeRef to Handler from var_spec type; refs: {type_refs:?}"
    );
}

// ---- field_declaration with qualified embedded type → Field symbol ---------

#[test]
fn coverage_field_declaration_qualified_embedded_type_emits_field() {
    // `sync.Mutex` as an embedded field — should emit a Field symbol and Inherits edge.
    let src = "package main\ntype Server struct { sync.Mutex\nName string }";
    let r = extract::extract(src);
    let field = r.symbols.iter().find(|s| s.name == "Mutex" && s.kind == SymbolKind::Field);
    assert!(
        field.is_some(),
        "expected Field symbol 'Mutex' from qualified embedded type; symbols: {:?}",
        r.symbols.iter().map(|s| (&s.name, &s.kind)).collect::<Vec<_>>()
    );
}

// ---- field_declaration: complex field types emit TypeRef for inner types ----

#[test]
fn coverage_field_declaration_slice_type_emits_type_ref() {
    // `Items []*Handler` — the `type_identifier "Handler"` inside the slice+pointer
    // type should produce a TypeRef.
    let src = "package main\ntype Server struct { Items []*Handler }";
    let r = extract::extract(src);
    let type_refs: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        type_refs.contains(&"Handler"),
        "expected TypeRef to Handler from []*Handler field; refs: {type_refs:?}"
    );
}

#[test]
fn coverage_field_declaration_nested_struct_emits_inner_fields() {
    // Anonymous inline struct fields must also emit Field symbols.
    let src = "package main\ntype Outer struct { Inner struct { A int } }";
    let r = extract::extract(src);
    let fields: Vec<_> = r
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Field)
        .map(|s| s.name.as_str())
        .collect();
    assert!(
        fields.contains(&"A"),
        "expected Field symbol 'A' from nested struct; fields: {fields:?}"
    );
}

// ---- var_spec with anonymous struct type → Field symbols -------------------

#[test]
fn coverage_var_spec_anonymous_struct_emits_field_symbols() {
    // `var opts struct{ Verbose bool }` — the field_declaration nodes inside the
    // anonymous struct type must produce Field symbols.
    let src = "package main\nvar opts struct {\n\tVerbose bool\n\tOutput string\n}";
    let r = extract::extract(src);
    let fields: Vec<&str> = r
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Field)
        .map(|s| s.name.as_str())
        .collect();
    assert!(
        fields.contains(&"Verbose"),
        "expected Field symbol 'Verbose' from anonymous struct in var_spec; fields: {fields:?}"
    );
    assert!(
        fields.contains(&"Output"),
        "expected Field symbol 'Output' from anonymous struct in var_spec; fields: {fields:?}"
    );
}

// ---- type_spec with type parameters (generics) → Struct symbol -------------

#[test]
fn coverage_type_spec_generic_struct_emits_struct_symbol() {
    // `type Result[T any] struct { Value T }` — the type_parameter_declaration
    // between the name and the struct_type must be skipped when identifying the
    // type body, so the symbol is still emitted as Struct (not TypeAlias).
    let src = "package main\ntype Result[T any] struct { Value T }";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "Result");
    assert!(
        sym.is_some(),
        "expected symbol 'Result' from generic type_spec; symbols: {:?}",
        r.symbols.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
    assert_eq!(
        sym.unwrap().kind,
        SymbolKind::Struct,
        "expected Struct kind for generic struct type_spec"
    );
}

// ---- type_declaration inside function body → Struct/Field symbols ----------

#[test]
fn coverage_type_declaration_in_function_body_emits_struct_and_fields() {
    // `type inner struct { X int }` declared inside a function body must produce
    // a Struct symbol and a Field symbol for X.
    let src = "package main\nfunc f() {\n\ttype inner struct { X int }\n\t_ = inner{}\n}";
    let r = extract::extract(src);
    let struct_sym = r.symbols.iter().find(|s| s.name == "inner" && s.kind == SymbolKind::Struct);
    assert!(
        struct_sym.is_some(),
        "expected Struct symbol 'inner' from type_declaration in function body; symbols: {:?}",
        r.symbols.iter().map(|s| (&s.name, &s.kind)).collect::<Vec<_>>()
    );
    let field_sym = r.symbols.iter().find(|s| s.name == "X" && s.kind == SymbolKind::Field);
    assert!(
        field_sym.is_some(),
        "expected Field symbol 'X' from type_declaration in function body; symbols: {:?}",
        r.symbols.iter().map(|s| (&s.name, &s.kind)).collect::<Vec<_>>()
    );
}
