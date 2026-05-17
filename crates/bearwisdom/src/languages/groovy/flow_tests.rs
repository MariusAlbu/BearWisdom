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
fn flow_def_local_chain_rhs_binds() {
    // `def x = obj.factory()` — RHS is a method chain, produces a Calls ref
    // inside the RHS byte range, so the flow engine can bind `x → ref_idx`.
    let src = r#"
class Bar {
    void test() {
        def client = HttpClientFactory.newClient()
        client.send(null)
    }
}
"#;
    let result = extract(src);
    let language: tree_sitter::Language = tree_sitter_groovy::LANGUAGE.into();
    let mut refs = result.refs;
    let meta = run_flow_queries(src, &language, &GROOVY_FLOW_CONFIG, &result.symbols, &mut refs);
    assert!(
        !meta.flow_binding_lhs.is_empty(),
        "expected flow binding for def-local with chain RHS"
    );
}

#[test]
fn flow_bare_assignment_chain_rhs_binds() {
    // `x = factory.create()` — bare reassignment where the RHS is a call chain.
    // The flow engine must emit a binding from `x` to the Calls ref for `create`.
    let src = r#"
class Baz {
    void test() {
        def repo
        repo = RepoFactory.create()
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
        "expected flow binding for bare assignment with chain RHS"
    );
}
