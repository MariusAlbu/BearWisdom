    use super::*;

    #[test]
    fn class_and_method_qualified_name() {
        let source = r#"class Foo:
    def bar(self):
        pass
"#;
        let r = extract(source);
        let cls = r.symbols.iter().find(|s| s.name == "Foo");
        assert!(cls.is_some(), "Expected 'Foo' class");
        assert_eq!(cls.unwrap().kind, SymbolKind::Class);

        let method = r.symbols.iter().find(|s| s.name == "bar");
        assert!(method.is_some(), "Expected 'bar' method");
        assert_eq!(method.unwrap().qualified_name, "Foo.bar");
        assert_eq!(method.unwrap().kind, SymbolKind::Method);
    }

    #[test]
    fn import_from_produces_ref_with_module() {
        let source = "from os.path import join\n";
        let r = extract(source);
        let import_refs: Vec<_> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Imports)
            .collect();
        assert!(!import_refs.is_empty(), "Expected import ref");
        let join_ref = import_refs
            .iter()
            .find(|r| r.target_name == "join")
            .expect("Expected 'join' ref");
        assert_eq!(
            join_ref.module.as_deref(),
            Some("os.path"),
            "Expected module 'os.path'"
        );
    }

    #[test]
    fn init_produces_constructor_kind() {
        let source = r#"class Foo:
    def __init__(self):
        pass
"#;
        let r = extract(source);
        let init = r
            .symbols
            .iter()
            .find(|s| s.name == "__init__")
            .expect("no __init__");
        assert_eq!(init.kind, SymbolKind::Constructor);
    }

    #[test]
    fn property_decorator_produces_property_kind() {
        let source = r#"class Foo:
    @property
    def name(self):
        return self._name
"#;
        let r = extract(source);
        let prop = r.symbols.iter().find(|s| s.name == "name").expect("no name");
        assert_eq!(prop.kind, SymbolKind::Property);
    }

    #[test]
    fn test_prefix_produces_test_kind() {
        let source = r#"def test_something():
    assert True
"#;
        let r = extract(source);
        let sym = r.symbols.iter().find(|s| s.name == "test_something").unwrap();
        assert_eq!(sym.kind, SymbolKind::Test);
    }

    #[test]
    fn private_visibility_for_underscore_names() {
        let source = "def _helper():\n    pass\n";
        let r = extract(source);
        let sym = r.symbols.iter().find(|s| s.name == "_helper").unwrap();
        assert_eq!(sym.visibility, Some(Visibility::Private));
    }

    #[test]
    fn extracts_docstring() {
        let source = r#"def documented():
    """This is the docstring."""
    pass
"#;
        let r = extract(source);
        assert_eq!(r.symbols.len(), 1);
        let doc = r.symbols[0].doc_comment.as_deref().unwrap_or("");
        assert!(doc.contains("docstring"), "doc_comment was: {doc:?}");
    }

    #[test]
    fn extracts_call_references() {
        let source = r#"def my_func():
    foo()
    bar.baz()
"#;
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
    fn class_inheritance_produces_type_refs() {
        let source = r#"class Foo(Bar, Baz):
    pass
"#;
        let r = extract(source);
        let type_refs: Vec<_> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::TypeRef)
            .collect();
        let names: Vec<&str> = type_refs.iter().map(|r| r.target_name.as_str()).collect();
        assert!(names.contains(&"Bar"), "Missing 'Bar': {names:?}");
        assert!(names.contains(&"Baz"), "Missing 'Baz': {names:?}");
    }

    #[test]
    fn handles_parse_errors_gracefully() {
        let source = "def broken(\nclass orphan\n    pass\n{{{";
        let result = std::panic::catch_unwind(|| extract(source));
        assert!(result.is_ok(), "extractor panicked on malformed input");
    }
