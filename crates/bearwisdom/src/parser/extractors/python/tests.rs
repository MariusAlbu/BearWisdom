    use super::*;
    use crate::types::{EdgeKind, SymbolKind, Visibility};

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

    // -----------------------------------------------------------------------
    // isinstance type narrowing
    // -----------------------------------------------------------------------

    #[test]
    fn isinstance_single_type_emits_type_ref() {
        let source = r#"
def check(user):
    if isinstance(user, Admin):
        user.admin_method()
"#;
        let r = extract(source);
        let type_refs: Vec<_> = r.refs.iter().filter(|r| r.kind == EdgeKind::TypeRef).collect();
        assert!(
            type_refs.iter().any(|r| r.target_name == "Admin"),
            "expected TypeRef to Admin, refs: {type_refs:?}"
        );
    }

    #[test]
    fn isinstance_tuple_of_types_emits_multiple_type_refs() {
        let source = r#"
def check(user):
    if isinstance(user, (Admin, Moderator)):
        user.privileged_method()
"#;
        let r = extract(source);
        let type_refs: Vec<_> = r.refs.iter().filter(|r| r.kind == EdgeKind::TypeRef).collect();
        let names: Vec<&str> = type_refs.iter().map(|r| r.target_name.as_str()).collect();
        assert!(names.contains(&"Admin"), "expected TypeRef to Admin, got: {names:?}");
        assert!(names.contains(&"Moderator"), "expected TypeRef to Moderator, got: {names:?}");
    }

    // -----------------------------------------------------------------------
    // With statement / context managers
    // -----------------------------------------------------------------------

    #[test]
    fn with_statement_alias_emits_variable_symbol() {
        let source = r#"
def read_file():
    with open('file.txt') as f:
        content = f.read()
"#;
        let r = extract(source);
        let var = r.symbols.iter().find(|s| s.name == "f" && s.kind == SymbolKind::Variable);
        assert!(var.is_some(), "expected 'f' Variable from with alias, symbols: {:?}", r.symbols);
    }

    #[test]
    fn with_statement_call_produces_chain_type_ref() {
        let source = r#"
def use_session(db):
    with db.session() as session:
        session.query(User)
"#;
        let r = extract(source);
        let type_refs: Vec<_> = r.refs.iter().filter(|r| r.kind == EdgeKind::TypeRef).collect();
        // Should have a chain TypeRef for db.session()
        assert!(
            type_refs.iter().any(|r| r.target_name == "session"),
            "expected chain TypeRef for db.session(), got: {type_refs:?}"
        );
    }

    #[test]
    fn with_statement_body_calls_extracted() {
        let source = r#"
def read_file():
    with open('file.txt') as f:
        content = f.read()
"#;
        let r = extract(source);
        let call_names: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(
            call_names.contains(&"open") || call_names.contains(&"read"),
            "expected calls from with body, got: {call_names:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Comprehensions
    // -----------------------------------------------------------------------

    #[test]
    fn list_comprehension_loop_var_emits_symbol() {
        let source = r#"
def transform(users):
    names = [u.name for u in users]
    return names
"#;
        let r = extract(source);
        let var = r.symbols.iter().find(|s| s.name == "u" && s.kind == SymbolKind::Variable);
        assert!(var.is_some(), "expected 'u' Variable from list comprehension, symbols: {:?}", r.symbols);
    }

    #[test]
    fn dict_comprehension_loop_var_emits_symbol() {
        let source = r#"
def make_map(users):
    user_map = {u.id: u for u in users}
    return user_map
"#;
        let r = extract(source);
        let var = r.symbols.iter().find(|s| s.name == "u" && s.kind == SymbolKind::Variable);
        assert!(var.is_some(), "expected 'u' Variable from dict comprehension, symbols: {:?}", r.symbols);
    }

    #[test]
    fn comprehension_body_calls_extracted() {
        let source = r#"
def transform(users):
    return [u.get_name() for u in users]
"#;
        let r = extract(source);
        let call_names: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(
            call_names.contains(&"get_name"),
            "expected get_name call from comprehension body, got: {call_names:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Walrus operator (:=)
    // -----------------------------------------------------------------------

    #[test]
    fn walrus_operator_emits_variable_symbol() {
        let source = r#"
def process(id):
    if (user := find_user(id)) is not None:
        user.process()
"#;
        let r = extract(source);
        let var = r.symbols.iter().find(|s| s.name == "user" && s.kind == SymbolKind::Variable);
        assert!(var.is_some(), "expected 'user' Variable from walrus, symbols: {:?}", r.symbols);
    }

    #[test]
    fn walrus_operator_chain_type_ref() {
        let source = r#"
def process(repo, id):
    if (user := repo.find_one(id)) is not None:
        user.process()
"#;
        let r = extract(source);
        let type_refs: Vec<_> = r.refs.iter().filter(|r| r.kind == EdgeKind::TypeRef).collect();
        assert!(
            type_refs.iter().any(|r| r.target_name == "find_one"),
            "expected chain TypeRef for repo.find_one(), got: {type_refs:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Match statement (Python 3.10+)
    // -----------------------------------------------------------------------

    #[test]
    fn match_class_pattern_emits_type_ref() {
        let source = r#"
def handle(command):
    match command:
        case User(name=n):
            process(n)
        case Admin():
            pass
"#;
        let r = extract(source);
        let type_refs: Vec<_> = r.refs.iter().filter(|r| r.kind == EdgeKind::TypeRef).collect();
        let names: Vec<&str> = type_refs.iter().map(|r| r.target_name.as_str()).collect();
        assert!(
            names.contains(&"User") || names.contains(&"Admin"),
            "expected TypeRef to User or Admin from match, got: {names:?}"
        );
    }

    #[test]
    fn match_as_pattern_emits_variable() {
        let source = r#"
def handle(command):
    match command:
        case Admin() as admin:
            admin.escalate()
"#;
        let r = extract(source);
        let var = r.symbols.iter().find(|s| s.name == "admin" && s.kind == SymbolKind::Variable);
        assert!(var.is_some(), "expected 'admin' Variable from as_pattern, symbols: {:?}", r.symbols);
    }

    // -----------------------------------------------------------------------
    // Lambda expressions
    // -----------------------------------------------------------------------

    #[test]
    fn lambda_params_emit_variable_symbols() {
        let source = r#"
def make_handler():
    handler = lambda x, y: x + y
    return handler
"#;
        let r = extract(source);
        let var_names: Vec<&str> = r
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Variable)
            .map(|s| s.name.as_str())
            .collect();
        assert!(
            var_names.contains(&"x") || var_names.contains(&"y"),
            "expected lambda param symbols, got: {var_names:?}"
        );
    }

    #[test]
    fn lambda_body_calls_extracted() {
        let source = r#"
def make_sorter():
    users_sorted = sorted(users, key=lambda u: u.get_name())
    return users_sorted
"#;
        let r = extract(source);
        let call_names: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(
            call_names.contains(&"get_name"),
            "expected get_name call from lambda body, got: {call_names:?}"
        );
    }
