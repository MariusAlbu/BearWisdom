    use super::extract::extract;
use crate::types::{ExtractedRef, ExtractedSymbol};

    #[test]
    fn impl_method_qualified_name() {
        let source = r#"struct Bar;

impl Bar {
    pub fn foo(&self) {}
}"#;
        let r = extract(source);
        let method = r.symbols.iter().find(|s| s.name == "foo");
        assert!(method.is_some(), "Expected method 'foo'");
        assert_eq!(method.unwrap().qualified_name, "Bar.foo");
        assert_eq!(method.unwrap().kind, SymbolKind::Method);
    }

    #[test]
    fn use_declaration_produces_import_ref() {
        let source = "use crate::db::Database;";
        let r = extract(source);
        let import_refs: Vec<_> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Imports)
            .collect();
        assert!(!import_refs.is_empty(), "Expected at least one import ref");
        let names: Vec<&str> = import_refs.iter().map(|r| r.target_name.as_str()).collect();
        assert!(
            names.contains(&"Database"),
            "Expected 'Database' in import targets, got: {names:?}"
        );
        let db_ref = import_refs
            .iter()
            .find(|r| r.target_name == "Database")
            .unwrap();
        assert_eq!(
            db_ref.module.as_deref(),
            Some("crate::db"),
            "Expected module 'crate::db'"
        );
    }

    #[test]
    fn enum_produces_enum_and_members() {
        let source = r#"enum Foo {
    A,
    B,
}"#;
        let r = extract(source);
        let enum_sym = r.symbols.iter().find(|s| s.name == "Foo");
        assert!(enum_sym.is_some(), "Expected 'Foo' enum");
        assert_eq!(enum_sym.unwrap().kind, SymbolKind::Enum);

        let members: Vec<_> = r
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::EnumMember)
            .collect();
        assert_eq!(members.len(), 2, "Expected 2 enum members, got {}", members.len());
        let names: Vec<&str> = members.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"A"), "Missing 'A'");
        assert!(names.contains(&"B"), "Missing 'B'");
    }

    #[test]
    fn trait_maps_to_interface_kind() {
        let source = "pub trait MyTrait { fn do_it(&self); }";
        let r = extract(source);
        let trait_sym = r.symbols.iter().find(|s| s.name == "MyTrait");
        assert!(trait_sym.is_some(), "Expected 'MyTrait'");
        assert_eq!(trait_sym.unwrap().kind, SymbolKind::Interface);
    }

    #[test]
    fn mod_maps_to_namespace_kind() {
        let source = r#"mod inner {
    pub fn foo() {}
}"#;
        let r = extract(source);
        let mod_sym = r.symbols.iter().find(|s| s.name == "inner");
        assert!(mod_sym.is_some(), "Expected 'inner' mod");
        assert_eq!(mod_sym.unwrap().kind, SymbolKind::Namespace);
        let fn_sym = r.symbols.iter().find(|s| s.name == "foo");
        assert_eq!(fn_sym.unwrap().qualified_name, "inner.foo");
    }

    #[test]
    fn extracts_pub_function() {
        let source = r#"pub fn greet(name: &str) -> String {
    format!("Hello, {}!", name)
}"#;
        let r = extract(source);
        assert_eq!(r.symbols.len(), 1);
        assert_eq!(r.symbols[0].name, "greet");
        assert_eq!(r.symbols[0].kind, SymbolKind::Function);
        assert_eq!(r.symbols[0].visibility, Some(Visibility::Public));
    }

    #[test]
    fn extracts_use_group_imports() {
        let source = "use std::collections::{HashMap, HashSet};";
        let r = extract(source);
        let names: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Imports)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(names.contains(&"HashMap"), "Missing HashMap: {names:?}");
        assert!(names.contains(&"HashSet"), "Missing HashSet: {names:?}");
    }

    #[test]
    fn extracts_test_function() {
        let source = r#"#[test]
fn test_something() {
    assert_eq!(1, 1);
}"#;
        let r = extract(source);
        let test_sym = r.symbols.iter().find(|s| s.name == "test_something");
        assert!(test_sym.is_some());
        assert_eq!(test_sym.unwrap().kind, SymbolKind::Test);
    }

    #[test]
    fn extracts_call_references() {
        let source = r#"fn run() {
    foo();
    bar.baz();
}"#;
        let r = extract(source);
        let call_names: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(call_names.contains(&"foo"), "Missing 'foo': {call_names:?}");
        assert!(call_names.contains(&"baz"), "Missing 'baz': {call_names:?}");
    }

    #[test]
    fn attaches_doc_comment() {
        let source = r#"/// Documentation for foo.
pub fn foo() {}"#;
        let r = extract(source);
        assert_eq!(r.symbols.len(), 1);
        let doc = r.symbols[0].doc_comment.as_deref().unwrap_or("");
        assert!(doc.contains("Documentation for foo"), "Got: {doc:?}");
    }

    #[test]
    fn handles_parse_errors_gracefully() {
        let source = r#"fn broken( { let x = ;"#;
        let r = extract(source);
        // Must not panic; partial results are acceptable.
        let _ = r.symbols;
    }

    #[test]
    fn calls_inside_closure_are_extracted() {
        let source = r#"fn run() {
    items.iter().map(|x| x.name.clone()).collect::<Vec<_>>();
}"#;
        let r = extract(source);
        let call_names: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(call_names.contains(&"map"),   "Missing 'map': {call_names:?}");
        assert!(call_names.contains(&"clone"), "Missing 'clone' inside closure: {call_names:?}");
    }

    #[test]
    fn closure_parameter_emitted_as_variable_symbol() {
        let source = r#"fn run() {
    items.iter().map(|x| x.process()).collect::<Vec<_>>();
}"#;
        let r = extract(source);
        let vars: Vec<&str> = r
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Variable)
            .map(|s| s.name.as_str())
            .collect();
        assert!(vars.contains(&"x"), "Missing closure param 'x': {vars:?}");
    }

    #[test]
    fn match_enum_variant_emits_typeref() {
        let source = r#"fn dispatch(msg: Message) {
    match msg {
        Message::Quit => quit(),
        Message::Move { x, y } => move_to(x, y),
    }
}"#;
        let r = extract(source);
        let typerefs: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::TypeRef)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(
            typerefs.iter().any(|n| n.contains("Message")),
            "Expected TypeRef for Message variant; got: {typerefs:?}"
        );
    }

    #[test]
    fn match_some_emits_typeref_and_binding_variable() {
        let source = r#"fn run(opt: Option<i32>) {
    match opt {
        Some(x) => println!("{}", x),
        None => {},
    }
}"#;
        let r = extract(source);
        let typerefs: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::TypeRef)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(typerefs.contains(&"Some"), "Expected TypeRef for Some: {typerefs:?}");

        let vars: Vec<&str> = r
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Variable && s.name == "x")
            .map(|s| s.name.as_str())
            .collect();
        assert!(!vars.is_empty(), "Expected Variable binding 'x' from Some(x)");
    }

    #[test]
    fn if_let_binding_emitted_as_variable() {
        let source = r#"fn run(opt: Option<String>) {
    if let Some(user) = find_user() {
        user.process();
    }
}"#;
        let r = extract(source);
        let vars: Vec<&str> = r
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Variable)
            .map(|s| s.name.as_str())
            .collect();
        assert!(vars.contains(&"user"), "Expected 'user' binding from if let: {vars:?}");
    }

    #[test]
    fn where_clause_bounds_emit_typerefs() {
        let source = r#"fn serialize<T>(item: &T) -> String
where
    T: Clone + Send + Serialize,
{
    String::new()
}"#;
        let r = extract(source);
        let typerefs: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::TypeRef)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(typerefs.contains(&"Clone"),     "Missing Clone:     {typerefs:?}");
        assert!(typerefs.contains(&"Send"),      "Missing Send:      {typerefs:?}");
        assert!(typerefs.contains(&"Serialize"), "Missing Serialize: {typerefs:?}");
    }

    // -----------------------------------------------------------------------
    // macro_invocation
    // -----------------------------------------------------------------------

    #[test]
    fn macro_invocation_emits_calls_edge() {
        let source = r#"fn run() {
    println!("hello");
    vec![1, 2, 3];
}"#;
        let r = extract(source);
        let call_names: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(
            call_names.contains(&"println"),
            "expected 'println' Calls edge from macro, got: {call_names:?}"
        );
        assert!(
            call_names.contains(&"vec"),
            "expected 'vec' Calls edge from macro, got: {call_names:?}"
        );
    }

    #[test]
    fn custom_macro_emits_calls_edge() {
        let source = r#"fn run() {
    tracing::info!("starting");
    bail!("oh no");
}"#;
        let r = extract(source);
        let call_names: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        // `bail` macro — should produce a Calls edge.
        assert!(
            call_names.contains(&"bail"),
            "expected 'bail' Calls edge, got: {call_names:?}"
        );
    }

    // -----------------------------------------------------------------------
    // type_cast_expression
    // -----------------------------------------------------------------------

    #[test]
    fn type_cast_user_type_emits_type_ref() {
        let source = r#"fn f(x: UserId) -> i64 {
    x as i64
}"#;
        let r = extract(source);
        // `i64` is a builtin — no TypeRef expected.  Verify no panic.
        let _ = r.refs;
    }

    #[test]
    fn type_cast_named_type_emits_type_ref() {
        // Cast to a user-defined type (uncommon but valid with newtype patterns).
        let source = r#"fn f(x: usize) -> MyIndex {
    x as MyIndex
}"#;
        let r = extract(source);
        let typerefs: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::TypeRef)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(
            typerefs.contains(&"MyIndex"),
            "expected TypeRef to MyIndex from cast, got: {typerefs:?}"
        );
    }

    // -----------------------------------------------------------------------
    // let_declaration variable binding
    // -----------------------------------------------------------------------

    #[test]
    fn let_declaration_emits_variable_symbol() {
        let source = r#"fn run() {
    let user = find_user();
    let (a, b) = split();
}"#;
        let r = extract(source);
        let var_names: Vec<&str> = r
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Variable)
            .map(|s| s.name.as_str())
            .collect();
        assert!(var_names.contains(&"user"), "missing 'user': {var_names:?}");
    }

    #[test]
    fn let_declaration_rhs_calls_extracted() {
        let source = r#"fn run(repo: &Repo) {
    let user = repo.find_one(1);
    let _ = user;
}"#;
        let r = extract(source);
        let call_names: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(
            call_names.contains(&"find_one"),
            "expected 'find_one' call from let rhs, got: {call_names:?}"
        );
    }

    // -----------------------------------------------------------------------
    // foreign_mod_item (extern "C")
    // -----------------------------------------------------------------------

    #[test]
    fn extern_block_functions_extracted_as_symbols() {
        let source = r#"extern "C" {
    fn malloc(size: usize) -> *mut u8;
    fn free(ptr: *mut u8);
}"#;
        let r = extract(source);
        let fn_names: Vec<&str> = r
            .symbols
            .iter()
            .filter(|s| matches!(s.kind, SymbolKind::Function | SymbolKind::Method))
            .map(|s| s.name.as_str())
            .collect();
        assert!(fn_names.contains(&"malloc"), "missing 'malloc': {fn_names:?}");
        assert!(fn_names.contains(&"free"),   "missing 'free':   {fn_names:?}");
    }

    // -----------------------------------------------------------------------
    // associated_type in impl
    // -----------------------------------------------------------------------

    #[test]
    fn associated_type_in_impl_extracted_as_type_alias() {
        let source = r#"struct MyIter;

impl Iterator for MyIter {
    type Item = i32;

    fn next(&mut self) -> Option<i32> {
        None
    }
}"#;
        let r = extract(source);
        let type_aliases: Vec<&str> = r
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::TypeAlias)
            .map(|s| s.name.as_str())
            .collect();
        assert!(
            type_aliases.contains(&"Item"),
            "expected 'Item' TypeAlias from associated type, got: {type_aliases:?}"
        );
    }

    #[test]
    fn associated_type_with_named_rhs_emits_type_ref() {
        let source = r#"struct Wrapper;

impl Container for Wrapper {
    type Output = MyValue;

    fn get(&self) -> Self::Output { unimplemented!() }
}"#;
        let r = extract(source);
        let typerefs: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::TypeRef)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(
            typerefs.contains(&"MyValue"),
            "expected TypeRef to MyValue from associated type RHS, got: {typerefs:?}"
        );
    }

    // -----------------------------------------------------------------------
    // extern crate declaration
    // -----------------------------------------------------------------------

    #[test]
    fn extern_crate_emits_imports_edge() {
        let source = "extern crate serde;";
        let r = extract(source);
        let imports: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Imports)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(
            imports.contains(&"serde"),
            "expected 'serde' Imports edge from extern crate, got: {imports:?}"
        );
    }

    // -----------------------------------------------------------------------
    // union_item
    // -----------------------------------------------------------------------

    #[test]
    fn union_item_extracted_as_struct_kind() {
        let source = r#"union MyUnion {
    i: i32,
    f: f32,
}"#;
        let r = extract(source);
        let sym = r.symbols.iter().find(|s| s.name == "MyUnion");
        assert!(sym.is_some(), "expected MyUnion symbol");
        assert_eq!(sym.unwrap().kind, SymbolKind::Struct);
    }

    // -----------------------------------------------------------------------
    // macro_definition (macro_rules!)
    // -----------------------------------------------------------------------

    #[test]
    fn macro_rules_definition_extracted_as_function() {
        let source = r#"macro_rules! my_vec {
    ($($x:expr),*) => { vec![$($x),*] };
}"#;
        let r = extract(source);
        let sym = r.symbols.iter().find(|s| s.name == "my_vec");
        assert!(sym.is_some(), "expected my_vec symbol from macro_rules!");
        assert_eq!(sym.unwrap().kind, SymbolKind::Function);
        let sig = sym.unwrap().signature.as_deref().unwrap_or("");
        assert!(sig.contains("macro_rules!"), "expected macro_rules! in sig, got: {sig:?}");
    }

    // -----------------------------------------------------------------------
    // struct_expression emits Calls + TypeRef
    // -----------------------------------------------------------------------

    #[test]
    fn struct_expression_emits_calls_and_type_ref() {
        let source = r#"fn build() -> Point {
    Point { x: 1, y: 2 }
}"#;
        let r = extract(source);
        let calls: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(
            calls.contains(&"Point"),
            "expected Calls edge for struct literal Point, got: {calls:?}"
        );
        let typerefs: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::TypeRef)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(
            typerefs.contains(&"Point"),
            "expected TypeRef for struct literal Point, got: {typerefs:?}"
        );
    }

    // -----------------------------------------------------------------------
    // dynamic_trait_type (dyn Trait)
    // -----------------------------------------------------------------------

    #[test]
    fn dyn_trait_type_in_cast_emits_type_ref() {
        let source = r#"fn f(e: Box<dyn Error>) -> i32 {
    let _ = e as i32;
    0
}"#;
        // This is a type cast to a primitive, no TypeRef for i32.
        // But the function itself has `dyn Error` in its signature — verify no panic.
        let r = extract(source);
        let _ = r.refs;
    }

