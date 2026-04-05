    use super::*;
    use crate::types::{EdgeKind, SymbolKind, Visibility};

    #[test]
    fn class_and_method_qualified_name() {
        let source = r#"class Foo:
    def bar(self):
        pass
"#;
        let r = extract::extract(source);
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
        let r = extract::extract(source);
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
        let r = extract::extract(source);
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
        let r = extract::extract(source);
        let prop = r.symbols.iter().find(|s| s.name == "name").expect("no name");
        assert_eq!(prop.kind, SymbolKind::Property);
    }

    #[test]
    fn test_prefix_produces_test_kind() {
        let source = r#"def test_something():
    assert True
"#;
        let r = extract::extract(source);
        let sym = r.symbols.iter().find(|s| s.name == "test_something").unwrap();
        assert_eq!(sym.kind, SymbolKind::Test);
    }

    #[test]
    fn private_visibility_for_underscore_names() {
        let source = "def _helper():\n    pass\n";
        let r = extract::extract(source);
        let sym = r.symbols.iter().find(|s| s.name == "_helper").unwrap();
        assert_eq!(sym.visibility, Some(Visibility::Private));
    }

    #[test]
    fn extracts_docstring() {
        let source = r#"def documented():
    """This is the docstring."""
    pass
"#;
        let r = extract::extract(source);
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
        let r = extract::extract(source);
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
        let r = extract::extract(source);
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
        let result = std::panic::catch_unwind(|| extract::extract(source));
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
        let r = extract::extract(source);
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
        let r = extract::extract(source);
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
        let r = extract::extract(source);
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
        let r = extract::extract(source);
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
        let r = extract::extract(source);
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
        let r = extract::extract(source);
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
        let r = extract::extract(source);
        let var = r.symbols.iter().find(|s| s.name == "u" && s.kind == SymbolKind::Variable);
        assert!(var.is_some(), "expected 'u' Variable from dict comprehension, symbols: {:?}", r.symbols);
    }

    #[test]
    fn comprehension_body_calls_extracted() {
        let source = r#"
def transform(users):
    return [u.get_name() for u in users]
"#;
        let r = extract::extract(source);
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
        let r = extract::extract(source);
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
        let r = extract::extract(source);
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
        let r = extract::extract(source);
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
        let r = extract::extract(source);
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
        let r = extract::extract(source);
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
        let r = extract::extract(source);
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

    // -----------------------------------------------------------------------
    // F-string interpolation
    // -----------------------------------------------------------------------

    #[test]
    fn fstring_interpolation_calls_extracted() {
        let source = r#"
def greet(user):
    return f"Hello {user.get_name()}"
"#;
        let r = extract::extract(source);
        let call_names: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(
            call_names.contains(&"get_name"),
            "expected get_name call from f-string interpolation, got: {call_names:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Type alias statement (Python 3.12+)
    // -----------------------------------------------------------------------

    #[test]
    fn type_alias_statement_emits_type_alias_symbol() {
        let source = "type Point = tuple[int, int]\n";
        let r = extract::extract(source);
        let sym = r.symbols.iter().find(|s| s.name == "Point");
        assert!(sym.is_some(), "expected TypeAlias symbol 'Point', got: {:?}", r.symbols);
        assert_eq!(sym.unwrap().kind, SymbolKind::TypeAlias);
    }

    #[test]
    fn type_alias_statement_emits_type_refs() {
        let source = "type UserOrAdmin = User | Admin\n";
        let r = extract::extract(source);
        let type_refs: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::TypeRef)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(
            type_refs.contains(&"User") || type_refs.contains(&"Admin"),
            "expected TypeRef edges from type alias, got: {type_refs:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Raise statement
    // -----------------------------------------------------------------------

    #[test]
    fn raise_statement_emits_type_ref_for_exception() {
        let source = r#"
def validate(value):
    if value is None:
        raise ValueError("value must not be None")
"#;
        let r = extract::extract(source);
        let type_refs: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::TypeRef)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(
            type_refs.contains(&"ValueError"),
            "expected TypeRef to ValueError, got: {type_refs:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Augmented assignment
    // -----------------------------------------------------------------------

    #[test]
    fn augmented_assignment_member_access_emits_calls_edge() {
        let source = r#"
def increment(self):
    self.count += 1
"#;
        let r = extract::extract(source);
        let calls: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(
            calls.contains(&"count"),
            "expected Calls edge for self.count +=, got: {calls:?}"
        );
    }

    // -----------------------------------------------------------------------
    // For / async-for statements
    // -----------------------------------------------------------------------

    #[test]
    fn for_statement_loop_var_emits_variable() {
        let source = r#"
def process(items):
    for item in items:
        item.save()
"#;
        let r = extract::extract(source);
        let vars: Vec<&str> = r
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Variable)
            .map(|s| s.name.as_str())
            .collect();
        assert!(vars.contains(&"item"), "expected loop var 'item', got: {vars:?}");
    }

    #[test]
    fn for_statement_body_calls_extracted() {
        let source = r#"
def process(items):
    for item in items:
        item.save()
"#;
        let r = extract::extract(source);
        let calls: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(calls.contains(&"save"), "expected save() call from for body, got: {calls:?}");
    }

    #[test]
    fn async_for_statement_loop_var_emits_variable() {
        let source = r#"
async def process(stream):
    async for item in stream:
        await item.save()
"#;
        let r = extract::extract(source);
        let vars: Vec<&str> = r
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Variable)
            .map(|s| s.name.as_str())
            .collect();
        assert!(vars.contains(&"item"), "expected async loop var 'item', got: {vars:?}");
    }

    #[test]
    fn async_with_statement_alias_emits_variable() {
        let source = r#"
async def run(engine):
    async with engine.connect() as conn:
        conn.execute()
"#;
        let r = extract::extract(source);
        let vars: Vec<&str> = r
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Variable)
            .map(|s| s.name.as_str())
            .collect();
        assert!(vars.contains(&"conn"), "expected async with alias 'conn', got: {vars:?}");
    }

    // -----------------------------------------------------------------------
    // Conditional expression
    // -----------------------------------------------------------------------

    #[test]
    fn conditional_expression_calls_extracted() {
        let source = r#"
def pick(flag):
    return foo() if flag else bar()
"#;
        let r = extract::extract(source);
        let calls: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(calls.contains(&"foo"), "expected foo() in ternary, got: {calls:?}");
        assert!(calls.contains(&"bar"), "expected bar() in ternary, got: {calls:?}");
    }

    // -----------------------------------------------------------------------
    // Assert statement
    // -----------------------------------------------------------------------

    #[test]
    fn assert_statement_calls_extracted() {
        let source = r#"
def validate(obj):
    assert obj.is_valid(), "not valid"
"#;
        let r = extract::extract(source);
        let calls: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(calls.contains(&"is_valid"), "expected is_valid() from assert, got: {calls:?}");
    }

    // -----------------------------------------------------------------------
    // Yield expression
    // -----------------------------------------------------------------------

    #[test]
    fn yield_expression_calls_extracted() {
        let source = r#"
def gen():
    yield compute()
"#;
        let r = extract::extract(source);
        let calls: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(calls.contains(&"compute"), "expected compute() from yield, got: {calls:?}");
    }

    // -----------------------------------------------------------------------
    // Tuple / list / dictionary / binary operator expressions
    // -----------------------------------------------------------------------

    #[test]
    fn tuple_expression_calls_extracted() {
        let source = r#"
def make_pair():
    return (foo(), bar())
"#;
        let r = extract::extract(source);
        let calls: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(calls.contains(&"foo"), "expected foo() from tuple, got: {calls:?}");
        assert!(calls.contains(&"bar"), "expected bar() from tuple, got: {calls:?}");
    }

    #[test]
    fn binary_operator_calls_extracted() {
        let source = r#"
def combine():
    return left() + right()
"#;
        let r = extract::extract(source);
        let calls: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(calls.contains(&"left"), "expected left() from binary op, got: {calls:?}");
        assert!(calls.contains(&"right"), "expected right() from binary op, got: {calls:?}");
    }

    // -----------------------------------------------------------------------
    // Except clause — TypeRef + Variable binding
    // -----------------------------------------------------------------------

    #[test]
    fn except_clause_emits_type_ref_for_exception() {
        let source = r#"
def run():
    try:
        do_work()
    except ValueError as e:
        handle(e)
"#;
        let r = extract::extract(source);
        let type_refs: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::TypeRef)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(
            type_refs.contains(&"ValueError"),
            "expected TypeRef to ValueError, got: {type_refs:?}"
        );
    }

    #[test]
    fn except_clause_emits_variable_for_as_binding() {
        let source = r#"
def run():
    try:
        do_work()
    except RuntimeError as err:
        log(err)
"#;
        let r = extract::extract(source);
        let vars: Vec<&str> = r
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Variable)
            .map(|s| s.name.as_str())
            .collect();
        assert!(vars.contains(&"err"), "expected Variable 'err' from except as binding, got: {vars:?}");
    }

    #[test]
    fn except_clause_multi_exception_emits_all_type_refs() {
        let source = r#"
def run():
    try:
        do_work()
    except (TypeError, ValueError) as e:
        handle(e)
"#;
        let r = extract::extract(source);
        let type_refs: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::TypeRef)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(type_refs.contains(&"TypeError"), "expected TypeRef to TypeError: {type_refs:?}");
        assert!(type_refs.contains(&"ValueError"), "expected TypeRef to ValueError: {type_refs:?}");
    }

    // -----------------------------------------------------------------------
    // Untyped default / splat parameters
    // -----------------------------------------------------------------------

    #[test]
    fn default_parameter_emits_variable_symbol() {
        let source = r#"
def foo(x=5, y="hello"):
    pass
"#;
        let r = extract::extract(source);
        let vars: Vec<&str> = r
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Variable)
            .map(|s| s.name.as_str())
            .collect();
        assert!(vars.contains(&"x") || vars.contains(&"y"),
            "expected default param Variable symbols, got: {vars:?}");
    }

    #[test]
    fn splat_params_emit_variable_symbols() {
        let source = r#"
def log(*args, **kwargs):
    pass
"#;
        let r = extract::extract(source);
        let vars: Vec<&str> = r
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Variable)
            .map(|s| s.name.as_str())
            .collect();
        assert!(vars.contains(&"args"), "expected *args Variable, got: {vars:?}");
        assert!(vars.contains(&"kwargs"), "expected **kwargs Variable, got: {vars:?}");
    }

    // -----------------------------------------------------------------------
    // Match pattern extensions
    // -----------------------------------------------------------------------

    #[test]
    fn match_splat_pattern_emits_variable() {
        let source = r#"
def handle(items):
    match items:
        case [first, *rest]:
            process(first, rest)
"#;
        let r = extract::extract(source);
        let vars: Vec<&str> = r
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Variable)
            .map(|s| s.name.as_str())
            .collect();
        assert!(
            vars.contains(&"rest") || vars.contains(&"first"),
            "expected splat pattern Variable, got: {vars:?}"
        );
    }

    #[test]
    fn match_dict_pattern_value_binding_emits_variable() {
        let source = r#"
def handle(event):
    match event:
        case {"type": action, "data": payload}:
            process(action)
"#;
        let r = extract::extract(source);
        let vars: Vec<&str> = r
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Variable)
            .map(|s| s.name.as_str())
            .collect();
        assert!(
            vars.contains(&"action") || vars.contains(&"payload"),
            "expected dict pattern value bindings, got: {vars:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Module-level call coverage
    // -----------------------------------------------------------------------

    #[test]
    fn module_level_call_emits_calls_edge() {
        let source = "setup_logging()\n";
        let r = extract::extract(source);
        let calls: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(
            calls.contains(&"setup_logging"),
            "expected Calls edge for module-level call, got: {calls:?}"
        );
    }

    #[test]
    fn module_level_method_call_emits_calls_edge() {
        let source = "app.run(debug=True)\n";
        let r = extract::extract(source);
        let calls: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(
            calls.contains(&"run"),
            "expected Calls edge for module-level method call, got: {calls:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Type annotation at module level
    // -----------------------------------------------------------------------

    #[test]
    fn module_level_annotated_assignment_emits_type_ref() {
        let source = "items: List[str] = []\n";
        let r = extract::extract(source);
        let type_refs: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::TypeRef)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(
            type_refs.contains(&"List") || type_refs.contains(&"str"),
            "expected TypeRef from module-level annotation, got: {type_refs:?}"
        );
    }

    // -------------------------------------------------------------------------
    // Import-map module resolution on qualified call refs
    // -------------------------------------------------------------------------

    #[test]
    fn from_import_call_sets_module_on_qualified_ref() {
        // `Person.objects.filter(team=team)` — chain root `Person` was imported
        // from `posthog.models`, so the Calls ref should carry that module.
        let source = r#"
from posthog.models import Person

def get_persons(team):
    return Person.objects.filter(team=team)
"#;
        let r = extract::extract(source);
        let call_ref = r
            .refs
            .iter()
            .find(|r| r.kind == EdgeKind::Calls && r.target_name == "filter")
            .expect("Expected Calls ref for 'filter'");
        assert_eq!(
            call_ref.module.as_deref(),
            Some("posthog.models"),
            "Expected module 'posthog.models' on filter call, chain root is 'Person'"
        );
    }

    #[test]
    fn plain_import_call_sets_module_on_qualified_ref() {
        // `json.dumps(data)` — chain root `json` imported via `import json`.
        let source = r#"
import json

def serialise(data):
    return json.dumps(data)
"#;
        let r = extract::extract(source);
        let call_ref = r
            .refs
            .iter()
            .find(|r| r.kind == EdgeKind::Calls && r.target_name == "dumps")
            .expect("Expected Calls ref for 'dumps'");
        assert_eq!(
            call_ref.module.as_deref(),
            Some("json"),
            "Expected module 'json' on dumps call"
        );
    }

    #[test]
    fn import_dotted_module_call_sets_module_on_qualified_ref() {
        // `import os.path` then `os.path.join(...)` — chain root `os` maps to `os.path`.
        let source = r#"
import os.path

def build_path(a, b):
    return os.path.join(a, b)
"#;
        let r = extract::extract(source);
        let call_ref = r
            .refs
            .iter()
            .find(|r| r.kind == EdgeKind::Calls && r.target_name == "join")
            .expect("Expected Calls ref for 'join'");
        assert_eq!(
            call_ref.module.as_deref(),
            Some("os.path"),
            "Expected module 'os.path' on join call"
        );
    }

    #[test]
    fn import_alias_call_sets_module_on_qualified_ref() {
        // `import numpy as np` then `np.array(...)` — alias `np` maps to `numpy`.
        let source = r#"
import numpy as np

def make_array(data):
    return np.array(data)
"#;
        let r = extract::extract(source);
        let call_ref = r
            .refs
            .iter()
            .find(|r| r.kind == EdgeKind::Calls && r.target_name == "array")
            .expect("Expected Calls ref for 'array'");
        assert_eq!(
            call_ref.module.as_deref(),
            Some("numpy"),
            "Expected module 'numpy' on array call"
        );
    }

    #[test]
    fn unimported_call_has_no_module() {
        // A call where the chain root is not in any import should have module=None.
        let source = r#"
def foo():
    local_obj.bar()
"#;
        let r = extract::extract(source);
        let call_ref = r
            .refs
            .iter()
            .find(|r| r.kind == EdgeKind::Calls && r.target_name == "bar")
            .expect("Expected Calls ref for 'bar'");
        assert_eq!(
            call_ref.module, None,
            "Expected no module on call where root is not imported"
        );
    }

    // -----------------------------------------------------------------------
    // Local variable type inference from RHS constructors
    // -----------------------------------------------------------------------

    #[test]
    fn assignment_uppercase_call_emits_typeref_for_variable() {
        // `repo = UserRepository(db)` — uppercase call → TypeRef "UserRepository"
        let src = "def handle(db):\n    repo = UserRepository(db)\n    repo.find_by_id(1)\n";
        let r = extract::extract(src);

        let repo_sym = r.symbols.iter().enumerate().find(|(_, s)| s.name == "repo");
        assert!(repo_sym.is_some(), "Expected Variable symbol 'repo'");
        let (repo_idx, _) = repo_sym.unwrap();

        let typeref = r.refs.iter().find(|rf| {
            rf.source_symbol_index == repo_idx
                && rf.kind == crate::types::EdgeKind::TypeRef
                && rf.target_name == "UserRepository"
                && rf.chain.is_none()
                && rf.module.is_none()
        });
        assert!(
            typeref.is_some(),
            "Expected TypeRef 'UserRepository' from 'repo'; refs from repo_idx = {:?}",
            r.refs
                .iter()
                .filter(|rf| rf.source_symbol_index == repo_idx)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn assignment_lowercase_call_does_not_emit_typeref() {
        // `x = get_value()` — lowercase call → no constructor TypeRef.
        let src = "def process():\n    x = get_value()\n    x.use_it()\n";
        let r = extract::extract(src);

        let x_sym = r.symbols.iter().enumerate().find(|(_, s)| s.name == "x");
        assert!(x_sym.is_some(), "Expected Variable symbol 'x'");
        let (x_idx, _) = x_sym.unwrap();

        let bare_typeref = r.refs.iter().any(|rf| {
            rf.source_symbol_index == x_idx
                && rf.kind == crate::types::EdgeKind::TypeRef
                && rf.chain.is_none()
                && rf.module.is_none()
                && rf.target_name == "get_value"
        });
        assert!(
            !bare_typeref,
            "Should not emit TypeRef for lowercase call 'get_value'"
        );
    }

    #[test]
    fn assignment_factory_method_uppercase_emits_typeref() {
        // `service = UserService.create(db)` — uppercase object → TypeRef "UserService"
        let src = "def setup(db):\n    service = UserService.create(db)\n    service.run()\n";
        let r = extract::extract(src);

        let svc_sym = r.symbols.iter().enumerate().find(|(_, s)| s.name == "service");
        assert!(svc_sym.is_some(), "Expected Variable symbol 'service'");
        let (svc_idx, _) = svc_sym.unwrap();

        let typeref = r.refs.iter().find(|rf| {
            rf.source_symbol_index == svc_idx
                && rf.kind == crate::types::EdgeKind::TypeRef
                && rf.target_name == "UserService"
                && rf.chain.is_none()
        });
        assert!(
            typeref.is_some(),
            "Expected TypeRef 'UserService' from 'service'; refs = {:?}",
            r.refs
                .iter()
                .filter(|rf| rf.source_symbol_index == svc_idx)
                .collect::<Vec<_>>()
        );
    }
