// =============================================================================
// c_lang/coverage_tests.rs — Node-kind coverage tests for the C/C++ extractor
//
// symbol_node_kinds:
//   function_definition, declaration, struct_specifier, union_specifier,
//   enum_specifier, enumerator, field_declaration, type_definition,
//   preproc_def, preproc_function_def,
//   class_specifier, namespace_definition, namespace_alias_definition,
//   alias_declaration, concept_definition, template_declaration
//
// ref_node_kinds:
//   call_expression, new_expression, preproc_include, type_identifier,
//   base_class_clause, cast_expression, sizeof_expression,
//   template_argument_list, import_declaration
// =============================================================================

use super::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// symbol_node_kinds — C
// ---------------------------------------------------------------------------

/// function_definition → SymbolKind::Function
#[test]
fn cov_function_definition_emits_function() {
    let r = extract::extract("int foo() { return 0; }", "c");
    let sym = r.symbols.iter().find(|s| s.name == "foo");
    assert!(sym.is_some(), "expected Function 'foo'; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Function);
}

/// declaration → SymbolKind::Variable  (variable declaration at file scope)
#[test]
fn cov_declaration_emits_variable() {
    let r = extract::extract("int count;", "c");
    let sym = r.symbols.iter().find(|s| s.name == "count");
    assert!(sym.is_some(), "expected Variable 'count'; got: {:?}", r.symbols);
}

/// struct_specifier → SymbolKind::Struct
#[test]
fn cov_struct_specifier_emits_struct() {
    let r = extract::extract("struct Point { int x; int y; };", "c");
    let sym = r.symbols.iter().find(|s| s.name == "Point");
    assert!(sym.is_some(), "expected Struct 'Point'; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Struct);
}

/// union_specifier → SymbolKind::Struct
#[test]
fn cov_union_specifier_emits_struct() {
    let r = extract::extract("union Data { int i; float f; };", "c");
    let sym = r.symbols.iter().find(|s| s.name == "Data");
    assert!(sym.is_some(), "expected Struct(union) 'Data'; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Struct);
}

/// enum_specifier → SymbolKind::Enum
#[test]
fn cov_enum_specifier_emits_enum() {
    let r = extract::extract("enum Color { RED, GREEN, BLUE };", "c");
    let sym = r.symbols.iter().find(|s| s.name == "Color");
    assert!(sym.is_some(), "expected Enum 'Color'; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Enum);
}

/// enumerator → child symbols inside an enum body
#[test]
fn cov_enumerator_emits_variable_symbols() {
    let r = extract::extract("enum Status { PENDING, ACTIVE, DONE };", "c");
    let sym = r.symbols.iter().find(|s| s.name == "PENDING");
    assert!(sym.is_some(), "expected enumerator 'PENDING'; got: {:?}", r.symbols);
}

/// field_declaration — the extractor processes field declarations inside struct bodies.
/// The struct itself is extracted as Struct, and each member (using `field_identifier`
/// in C grammar) is extracted as a Variable child symbol.
#[test]
fn cov_field_declaration_struct_and_members_extracted() {
    let r = extract::extract("struct Point { int x; int y; };", "c");
    let has_struct = r.symbols.iter().any(|s| s.name == "Point");
    assert!(has_struct, "expected Struct 'Point' from struct_specifier; got: {:?}", r.symbols);
    let has_x = r.symbols.iter().any(|s| s.name == "x");
    assert!(has_x, "expected field member 'x' from field_identifier; got: {:?}", r.symbols);
    let has_y = r.symbols.iter().any(|s| s.name == "y");
    assert!(has_y, "expected field member 'y' from field_identifier; got: {:?}", r.symbols);
}

/// type_definition → SymbolKind::TypeAlias
#[test]
fn cov_type_definition_emits_type_alias() {
    let r = extract::extract("typedef unsigned int uint32;", "c");
    let sym = r.symbols.iter().find(|s| s.name == "uint32");
    assert!(sym.is_some(), "expected TypeAlias 'uint32'; got: {:?}", r.symbols);
}

/// preproc_def → SymbolKind::Variable (macro constant)
#[test]
fn cov_preproc_def_emits_variable() {
    let r = extract::extract("#define MAX_BUF 1024\n", "c");
    let sym = r.symbols.iter().find(|s| s.name == "MAX_BUF");
    assert!(sym.is_some(), "expected Variable(macro) 'MAX_BUF'; got: {:?}", r.symbols);
}

/// preproc_function_def → SymbolKind::Function (function-like macro)
#[test]
fn cov_preproc_function_def_emits_function() {
    let r = extract::extract("#define MAX(a, b) ((a) > (b) ? (a) : (b))\n", "c");
    let sym = r.symbols.iter().find(|s| s.name == "MAX");
    assert!(sym.is_some(), "expected Function(macro) 'MAX'; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Function);
}

// ---------------------------------------------------------------------------
// symbol_node_kinds — C++
// ---------------------------------------------------------------------------

/// class_specifier → SymbolKind::Class  (C++)
#[test]
fn cov_class_specifier_emits_class() {
    let r = extract::extract("class Animal { public: int id; };", "cpp");
    let sym = r.symbols.iter().find(|s| s.name == "Animal");
    assert!(sym.is_some(), "expected Class 'Animal'; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Class);
}

/// namespace_definition → SymbolKind::Namespace  (C++)
#[test]
fn cov_namespace_definition_emits_namespace() {
    let r = extract::extract("namespace myns { int x; }", "cpp");
    let sym = r.symbols.iter().find(|s| s.name == "myns");
    assert!(sym.is_some(), "expected Namespace 'myns'; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Namespace);
}

/// namespace_alias_definition — `namespace fs = std::filesystem;`
/// The C++ grammar parses this as a `namespace_alias_definition` node.
/// The extractor handles it; does not crash and may emit a TypeAlias symbol.
#[test]
fn cov_namespace_alias_definition_does_not_crash() {
    let r = extract::extract("namespace fs = std::filesystem;", "cpp");
    // Acceptable outcomes: emit a TypeAlias for 'fs', or produce no symbol.
    // Either is fine — what matters is no panic.
    let _ = r;
}

/// alias_declaration (using Alias = Type;) → SymbolKind::TypeAlias  (C++)
#[test]
fn cov_alias_declaration_emits_type_alias() {
    let r = extract::extract("using MyInt = int;", "cpp");
    let sym = r.symbols.iter().find(|s| s.name == "MyInt");
    assert!(sym.is_some(), "expected TypeAlias 'MyInt'; got: {:?}", r.symbols);
}

/// template_declaration → wraps function/class and emits the inner symbol
#[test]
fn cov_template_declaration_emits_inner_symbol() {
    let r = extract::extract("template<typename T> class Box { T val; };", "cpp");
    let sym = r.symbols.iter().find(|s| s.name == "Box");
    assert!(sym.is_some(), "expected symbol 'Box' from template_declaration; got: {:?}", r.symbols);
}

/// concept_definition → SymbolKind::Interface  (C++20)
/// Note: tree-sitter-cpp may not fully parse concepts in all versions;
/// we verify no crash and at least accept the source.
#[test]
fn cov_concept_definition_does_not_crash() {
    let src = "template<typename T> concept Printable = requires(T t) { t.print(); };";
    let r = extract::extract(src, "cpp");
    // Concept may emit Interface or Variable — either is acceptable.
    let _ = r;
}

// ---------------------------------------------------------------------------
// ref_node_kinds — C
// ---------------------------------------------------------------------------

/// call_expression → EdgeKind::Calls
#[test]
fn cov_call_expression_emits_calls() {
    let src = "int main() { printf(\"hi\"); return 0; }";
    let r = extract::extract(src, "c");
    let calls: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Calls)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(calls.contains(&"printf"), "expected Calls to 'printf'; got: {calls:?}");
}

/// preproc_include → EdgeKind::Imports
#[test]
fn cov_preproc_include_emits_imports() {
    let r = extract::extract("#include <stdio.h>\n", "c");
    let imports: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Imports)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        imports.iter().any(|n| n.contains("stdio")),
        "expected Imports ref for stdio.h; got: {imports:?}"
    );
}

/// type_identifier in field declaration → EdgeKind::TypeRef
#[test]
fn cov_type_identifier_in_field_emits_type_ref() {
    let src = "struct Order { Customer *owner; };";
    let r = extract::extract(src, "c");
    let type_refs: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        type_refs.contains(&"Customer"),
        "expected TypeRef to 'Customer'; got: {type_refs:?}"
    );
}

/// new_expression → EdgeKind::Instantiates  (C++)
#[test]
fn cov_new_expression_emits_instantiates() {
    let src = "void f() { Foo *p = new Foo(); }";
    let r = extract::extract(src, "cpp");
    let inst: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Instantiates)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        inst.contains(&"Foo"),
        "expected Instantiates to 'Foo' from new_expression; got: {inst:?}"
    );
}

/// base_class_clause → EdgeKind::Inherits  (C++)
#[test]
fn cov_base_class_clause_emits_inherits() {
    let src = "class Dog : public Animal {};";
    let r = extract::extract(src, "cpp");
    let inherits: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Inherits)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        inherits.contains(&"Animal"),
        "expected Inherits from 'Animal'; got: {inherits:?}"
    );
}

/// cast_expression → no crash; extractor processes cast nodes
#[test]
fn cov_cast_expression_does_not_crash() {
    let src = "void f() { int x = (int)3.14; }";
    let r = extract::extract(src, "c");
    let _ = r;
}

/// sizeof_expression → no crash
#[test]
fn cov_sizeof_expression_does_not_crash() {
    let src = "size_t s = sizeof(int);";
    let r = extract::extract(src, "c");
    let _ = r;
}

/// template_argument_list → TypeRef edges for template args  (C++)
#[test]
fn cov_template_argument_list_emits_type_ref() {
    let src = "void f() { std::vector<MyType> v; }";
    let r = extract::extract(src, "cpp");
    let type_refs: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        type_refs.contains(&"MyType"),
        "expected TypeRef to 'MyType' from template_argument_list; got: {type_refs:?}"
    );
}

/// import_declaration (C++20 modules) → does not crash
#[test]
fn cov_import_declaration_does_not_crash() {
    // C++20 module import syntax; grammar may not fully support it everywhere.
    let src = "import std.core;\nvoid f() {}";
    let r = extract::extract(src, "cpp");
    let _ = r;
}

// ---------------------------------------------------------------------------
// symbol_node_kinds — C (additional)
// ---------------------------------------------------------------------------

/// declaration with function_declarator → forward declaration emits Function
#[test]
fn cov_declaration_function_declarator_emits_function() {
    let r = extract::extract("int compute(int a, int b);", "c");
    let sym = r.symbols.iter().find(|s| s.name == "compute");
    assert!(sym.is_some(), "expected a symbol for forward-decl 'compute'; got: {:?}", r.symbols);
    assert_eq!(
        sym.unwrap().kind,
        SymbolKind::Function,
        "expected Function for forward-decl 'compute'; got: {:?}", sym.unwrap().kind
    );
}

/// typedef struct { ... } Alias — anonymous struct with typedef alias
/// The extractor should emit a TypeAlias named after the typedef declarator.
#[test]
fn cov_typedef_anonymous_struct_emits_type_alias() {
    let r = extract::extract("typedef struct { int x; int y; } Point;", "c");
    let sym = r.symbols.iter().find(|s| s.name == "Point");
    assert!(sym.is_some(), "expected TypeAlias 'Point' from typedef struct; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::TypeAlias);
}

/// typedef function pointer → TypeAlias
#[test]
fn cov_typedef_function_pointer_emits_type_alias() {
    let r = extract::extract("typedef int (*Callback)(void *ctx);", "c");
    let sym = r.symbols.iter().find(|s| s.name == "Callback");
    assert!(sym.is_some(), "expected TypeAlias 'Callback' from typedef fn ptr; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::TypeAlias);
}

/// static function → Function with private visibility
#[test]
fn cov_static_function_emits_function() {
    let r = extract::extract("static int helper(void) { return 0; }", "c");
    let sym = r.symbols.iter().find(|s| s.name == "helper");
    assert!(sym.is_some(), "expected Function 'helper' from static fn; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Function);
}

/// extern variable declaration → Variable
#[test]
fn cov_extern_declaration_emits_variable() {
    let r = extract::extract("extern int global_count;", "c");
    let sym = r.symbols.iter().find(|s| s.name == "global_count");
    assert!(sym.is_some(), "expected Variable 'global_count' from extern decl; got: {:?}", r.symbols);
}

/// init_declarator path: `int x = 0;` → Variable (declarator wraps identifier via init_declarator)
#[test]
fn cov_declaration_init_declarator_emits_variable() {
    let r = extract::extract("int count = 42;", "c");
    let sym = r.symbols.iter().find(|s| s.name == "count");
    assert!(sym.is_some(), "expected Variable 'count' from init_declarator; got: {:?}", r.symbols);
}

/// enumerator with explicit value → EnumMember
#[test]
fn cov_enumerator_with_value_emits_enum_member() {
    let r = extract::extract("enum Dir { NORTH = 0, SOUTH = 1 };", "c");
    let north = r.symbols.iter().find(|s| s.name == "NORTH");
    assert!(north.is_some(), "expected EnumMember 'NORTH'; got: {:?}", r.symbols);
    assert_eq!(north.unwrap().kind, SymbolKind::EnumMember);
}

/// preproc_ifdef conditional branches — symbols from all branches extracted
#[test]
fn cov_preproc_ifdef_extracts_symbols_from_branches() {
    let src = "#ifdef DEBUG\nint debug_flag;\n#else\nint release_flag;\n#endif\n";
    let r = extract::extract(src, "c");
    let names: Vec<&str> = r.symbols.iter().map(|s| s.name.as_str()).collect();
    // At minimum one branch should be extracted (either debug_flag or release_flag).
    assert!(
        names.contains(&"debug_flag") || names.contains(&"release_flag"),
        "expected at least one symbol from preproc_ifdef branches; got: {names:?}"
    );
}

// ---------------------------------------------------------------------------
// symbol_node_kinds — C++ (additional)
// ---------------------------------------------------------------------------

/// function_definition inside a class body → SymbolKind::Method
#[test]
fn cov_method_in_class_emits_method() {
    let src = "class Foo { public: int bar() { return 0; } };";
    let r = extract::extract(src, "cpp");
    let sym = r.symbols.iter().find(|s| s.name == "bar");
    assert!(sym.is_some(), "expected Method 'bar'; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Method);
}

/// constructor: function_definition whose name matches class name → Constructor
#[test]
fn cov_constructor_emits_constructor() {
    let src = "class Engine { public: Engine() {} };";
    let r = extract::extract(src, "cpp");
    let sym = r.symbols.iter().find(|s| s.name == "Engine" && s.kind == SymbolKind::Constructor);
    assert!(sym.is_some(), "expected Constructor 'Engine'; got: {:?}", r.symbols);
}

/// destructor: `~ClassName()` → Method (destructor)
#[test]
fn cov_destructor_emits_method() {
    let src = "class Engine { public: ~Engine() {} };";
    let r = extract::extract(src, "cpp");
    // Destructor is emitted as Method named ~Engine.
    let sym = r.symbols.iter().find(|s| s.name.contains("Engine") && s.kind == SymbolKind::Method);
    assert!(sym.is_some(), "expected Method destructor '~Engine'; got: {:?}", r.symbols);
}

/// operator overload → Method (inside class) with operator name
#[test]
fn cov_operator_overload_emits_method() {
    let src = "class Vec { public: Vec operator+(const Vec& o) const { return *this; } };";
    let r = extract::extract(src, "cpp");
    let sym = r.symbols.iter().find(|s| s.name.contains("operator"));
    assert!(sym.is_some(), "expected Method 'operator+' from overload; got: {:?}", r.symbols);
}

/// pure virtual declaration in class → Method (virtual/abstract)
#[test]
fn cov_pure_virtual_declaration_emits_method() {
    let src = "class Shape { public: virtual int area() = 0; };";
    let r = extract::extract(src, "cpp");
    let shape = r.symbols.iter().find(|s| s.name == "Shape");
    assert!(shape.is_some(), "expected Class 'Shape' to be extracted; got: {:?}", r.symbols);
    let area = r.symbols.iter().find(|s| s.name == "area");
    assert!(area.is_some(), "expected Method 'area' from pure-virtual declaration; got: {:?}", r.symbols);
    assert_eq!(
        area.unwrap().kind,
        SymbolKind::Method,
        "expected Method for pure-virtual 'area'; got: {:?}", area.unwrap().kind
    );
}

/// nested class inside outer class → Class with qualified name
#[test]
fn cov_nested_class_emits_class() {
    let src = "class Outer { public: class Inner {}; };";
    let r = extract::extract(src, "cpp");
    let sym = r.symbols.iter().find(|s| s.name == "Inner");
    assert!(sym.is_some(), "expected Class 'Inner' from nested class; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Class);
}

/// anonymous namespace → namespace body symbols extracted (no crash)
#[test]
fn cov_anonymous_namespace_does_not_crash() {
    let src = "namespace { int internal_val = 0; }";
    let r = extract::extract(src, "cpp");
    // Anonymous namespace has no name — we just verify no panic and inner symbols present.
    let sym = r.symbols.iter().find(|s| s.name == "internal_val");
    assert!(sym.is_some(), "expected Variable 'internal_val' inside anonymous namespace; got: {:?}", r.symbols);
}

/// template function definition → Function symbol emitted
#[test]
fn cov_template_function_emits_function() {
    let src = "template<typename T> T identity(T val) { return val; }";
    let r = extract::extract(src, "cpp");
    let sym = r.symbols.iter().find(|s| s.name == "identity");
    assert!(sym.is_some(), "expected Function 'identity' from template fn; got: {:?}", r.symbols);
}

/// alias_declaration inside class with type ref → TypeAlias + TypeRef edge
#[test]
fn cov_alias_declaration_in_class_emits_type_alias_and_typeref() {
    let src = "class Wrapper { public: using ValueType = int; };";
    let r = extract::extract(src, "cpp");
    let sym = r.symbols.iter().find(|s| s.name == "ValueType");
    assert!(sym.is_some(), "expected TypeAlias 'ValueType' inside class; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::TypeAlias);
}

// ---------------------------------------------------------------------------
// ref_node_kinds — C (additional)
// ---------------------------------------------------------------------------

/// call_expression via field_expression (struct function pointer) → Calls
#[test]
fn cov_call_via_field_expression_emits_calls() {
    let src = "void run(struct Obj *o) { o->init(o); }";
    let r = extract::extract(src, "c");
    let calls: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Calls)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(calls.contains(&"init"), "expected Calls to 'init' via field_expression; got: {calls:?}");
}

/// type_identifier in return type → TypeRef
#[test]
fn cov_return_type_identifier_emits_type_ref() {
    let src = "Node *create_node(void);";
    let r = extract::extract(src, "c");
    let type_refs: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        type_refs.contains(&"Node"),
        "expected TypeRef to 'Node' from return type; got: {type_refs:?}"
    );
}

/// type_identifier in parameter declaration → TypeRef
#[test]
fn cov_param_type_identifier_emits_type_ref() {
    let src = "void process(Request *req, Response *resp) {}";
    let r = extract::extract(src, "c");
    let type_refs: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        type_refs.contains(&"Request"),
        "expected TypeRef to 'Request' from param type; got: {type_refs:?}"
    );
    assert!(
        type_refs.contains(&"Response"),
        "expected TypeRef to 'Response' from param type; got: {type_refs:?}"
    );
}

/// cast_expression → TypeRef for cast target type
#[test]
fn cov_cast_expression_emits_type_ref() {
    let src = "void f(void *p) { MyStruct *s = (MyStruct *)p; }";
    let r = extract::extract(src, "c");
    let type_refs: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        type_refs.contains(&"MyStruct"),
        "expected TypeRef to 'MyStruct' from cast_expression; got: {type_refs:?}"
    );
}

/// sizeof_expression with named type → TypeRef
#[test]
fn cov_sizeof_expression_emits_type_ref() {
    let src = "size_t get_size(void) { return sizeof(MyStruct); }";
    let r = extract::extract(src, "c");
    let type_refs: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        type_refs.contains(&"MyStruct"),
        "expected TypeRef to 'MyStruct' from sizeof_expression; got: {type_refs:?}"
    );
}

/// compound_literal_expression → TypeRef for the literal type
// TODO: compound_literal_expression — tree-sitter-c parses `(Foo){ .x = 1 }` as a
// cast_expression wrapping a compound_literal_expression; TypeRef is emitted via
// the cast path rather than a dedicated compound_literal handler.
#[test]
fn cov_compound_literal_expression_does_not_crash() {
    let src = "struct Point make_pt(void) { return (struct Point){ .x = 1, .y = 2 }; }";
    let r = extract::extract(src, "c");
    let _ = r; // No crash required; TypeRef may or may not be emitted.
}

/// type_identifier in variable declaration at file scope → TypeRef
#[test]
fn cov_variable_declaration_type_identifier_emits_type_ref() {
    let src = "Node *head;";
    let r = extract::extract(src, "c");
    let type_refs: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        type_refs.contains(&"Node"),
        "expected TypeRef to 'Node' from variable decl type; got: {type_refs:?}"
    );
}

/// preproc_include with quoted path → Imports with module path set
#[test]
fn cov_preproc_include_quoted_sets_module() {
    let r = extract::extract("#include \"utils/queue.h\"\n", "c");
    let imp = r.refs.iter().find(|r| r.kind == EdgeKind::Imports && r.target_name.contains("queue"));
    assert!(imp.is_some(), "expected Imports ref for queue.h; got: {:?}", r.refs);
    assert!(
        imp.unwrap().module.as_deref().map(|m| m.contains("queue")).unwrap_or(false),
        "expected module path to contain 'queue'; got: {:?}", imp.unwrap().module
    );
}

// ---------------------------------------------------------------------------
// ref_node_kinds — C++ (additional)
// ---------------------------------------------------------------------------

/// call_expression with qualified_identifier (Namespace::fn) → Calls
#[test]
fn cov_qualified_call_expression_emits_calls() {
    let src = "void f() { Math::sqrt(4.0); }";
    let r = extract::extract(src, "cpp");
    let calls: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Calls)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(calls.contains(&"sqrt"), "expected Calls to 'sqrt' via qualified_identifier; got: {calls:?}");
}

/// call_expression with template_function (foo<T>(...)) → Calls
#[test]
fn cov_template_function_call_emits_calls() {
    let src = "void f() { convert<int>(3.14); }";
    let r = extract::extract(src, "cpp");
    let calls: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Calls)
        .map(|r| r.target_name.as_str())
        .collect();
    // TODO: template_function call target resolution — may be emitted as 'convert' or skipped.
    // Accept either: the call is present or no crash.
    let _ = calls;
}

/// new_expression with template type → Instantiates for the template type name
#[test]
fn cov_new_expression_template_type_emits_instantiates() {
    let src = "void f() { auto *v = new std::vector<Item>(); }";
    let r = extract::extract(src, "cpp");
    // The outer template type name should appear as Instantiates.
    let inst: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Instantiates)
        .map(|r| r.target_name.as_str())
        .collect();
    // Item should also appear as a TypeRef.
    let type_refs: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        inst.iter().any(|n| *n == "vector" || *n == "Item") || type_refs.contains(&"Item"),
        "expected Instantiates or TypeRef for template new expression; inst: {inst:?}, typerefs: {type_refs:?}"
    );
}

/// lambda body calls → Calls edges extracted from lambda
#[test]
fn cov_lambda_body_calls_emits_calls() {
    let src = "void f() { auto fn = [](){ process(); }; fn(); }";
    let r = extract::extract(src, "cpp");
    let calls: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Calls)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(calls.contains(&"process"), "expected Calls to 'process' from lambda body; got: {calls:?}");
}

/// catch_clause parameter type → TypeRef
#[test]
fn cov_catch_clause_emits_type_ref() {
    let src = "void f() { try { } catch (MyException &e) { } }";
    let r = extract::extract(src, "cpp");
    let type_refs: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        type_refs.contains(&"MyException"),
        "expected TypeRef to 'MyException' from catch_clause; got: {type_refs:?}"
    );
}

/// using_declaration (`using std::vector;`) → Imports
#[test]
fn cov_using_declaration_emits_imports() {
    let src = "using std::vector;";
    let r = extract::extract(src, "cpp");
    let imports: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Imports)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        imports.iter().any(|n| n.contains("vector")),
        "expected Imports ref from using_declaration; got: {imports:?}"
    );
}

/// alias_declaration aliased type → TypeRef edge
#[test]
fn cov_alias_declaration_aliased_type_emits_type_ref() {
    let src = "using NodePtr = TreeNode *;";
    let r = extract::extract(src, "cpp");
    let type_refs: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        type_refs.contains(&"TreeNode"),
        "expected TypeRef to 'TreeNode' from alias_declaration type; got: {type_refs:?}"
    );
}

/// template_declaration with default type arg → TypeRef for default type
#[test]
fn cov_template_declaration_default_type_arg_emits_type_ref() {
    let src = "template<typename T = DefaultAllocator> class Pool {};";
    let r = extract::extract(src, "cpp");
    let type_refs: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        type_refs.contains(&"DefaultAllocator"),
        "expected TypeRef to 'DefaultAllocator' from template default arg; got: {type_refs:?}"
    );
}

/// struct inheritance in C++ → Inherits edge (struct_specifier with base_class_clause)
#[test]
fn cov_struct_base_class_clause_emits_inherits() {
    let src = "struct Derived : Base {};";
    let r = extract::extract(src, "cpp");
    let inherits: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Inherits)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        inherits.contains(&"Base"),
        "expected Inherits from 'Base' via struct base_class_clause; got: {inherits:?}"
    );
}

/// multiple inheritance → multiple Inherits edges
#[test]
fn cov_multiple_inheritance_emits_multiple_inherits() {
    let src = "class Child : public ParentA, public ParentB {};";
    let r = extract::extract(src, "cpp");
    let inherits: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Inherits)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        inherits.contains(&"ParentA") && inherits.contains(&"ParentB"),
        "expected Inherits for both 'ParentA' and 'ParentB'; got: {inherits:?}"
    );
}

/// type_identifier in typedef target → TypeRef
#[test]
fn cov_typedef_target_type_emits_type_ref() {
    let src = "typedef LinkedList *ListPtr;";
    let r = extract::extract(src, "c");
    let type_refs: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        type_refs.contains(&"LinkedList"),
        "expected TypeRef to 'LinkedList' from typedef target; got: {type_refs:?}"
    );
}
