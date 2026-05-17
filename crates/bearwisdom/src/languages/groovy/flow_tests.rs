use super::flow::GROOVY_FLOW_CONFIG;
use crate::indexer::flow::run_flow_queries;
use crate::languages::groovy::extract::extract;

#[test]
fn flow_typed_local_binds_rhs() {
    let src = r#"
class Foo {
    void bar() {
        HttpClient client = HttpClient.newBuilder().build()
        client.send(null, null)
    }
}
"#;
    let result = extract(src);
    let language: tree_sitter::Language = tree_sitter_groovy::LANGUAGE.into();
    let mut refs = result.refs;
    let meta = run_flow_queries(src, &language, &GROOVY_FLOW_CONFIG, &result.symbols, &mut refs);
    // At least one flow binding must be produced for the `client =` declaration.
    assert!(
        !meta.flow_binding_lhs.is_empty(),
        "expected flow binding for typed local declaration"
    );
}

#[test]
fn flow_def_local_binds_rhs() {
    let src = r#"
class Bar {
    void test() {
        def svc = new MyService()
        svc.doWork()
    }
}
"#;
    let result = extract(src);
    let language: tree_sitter::Language = tree_sitter_groovy::LANGUAGE.into();
    let mut refs = result.refs;
    let meta = run_flow_queries(src, &language, &GROOVY_FLOW_CONFIG, &result.symbols, &mut refs);
    assert!(
        !meta.flow_binding_lhs.is_empty(),
        "expected flow binding for def-local declaration"
    );
}

#[test]
fn flow_bare_assignment_binds_rhs() {
    let src = r#"
class Baz {
    void test() {
        def repo
        repo = new UserRepository()
        repo.findAll()
    }
}
"#;
    let result = extract(src);
    let language: tree_sitter::Language = tree_sitter_groovy::LANGUAGE.into();
    let mut refs = result.refs;
    let meta = run_flow_queries(src, &language, &GROOVY_FLOW_CONFIG, &result.symbols, &mut refs);
    assert!(
        !meta.flow_binding_lhs.is_empty(),
        "expected flow binding for bare assignment expression"
    );
}
