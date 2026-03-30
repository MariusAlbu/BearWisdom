    use super::*;

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
