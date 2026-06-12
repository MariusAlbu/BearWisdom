// Tests for calls.rs — extract_call_args recursive CallArg construction.

use super::extract;
use crate::types::{CallArg, EdgeKind};

/// Parse a Groovy snippet and return the call_args of the first Calls ref that
/// carries non-empty arguments.
fn parse_call_args(src: &str) -> Vec<CallArg> {
    let result = extract::extract(src);
    result
        .refs
        .into_iter()
        .find(|r| r.kind == EdgeKind::Calls && !r.call_args.is_empty())
        .map(|r| r.call_args)
        .unwrap_or_default()
}

#[test]
fn call_args_string_literal() {
    let src = r#"
class Caller {
    void run() { fetch("api/users") }
}
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::StringLit(s) if s == "api/users")),
        "expected StringLit(\"api/users\"), got: {args:?}"
    );
}

#[test]
fn call_args_identifier_becomes_ident_variant() {
    let src = r#"
class Caller {
    void run(url) { fetch(url) }
}
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::Ident(s) if s == "url")),
        "expected Ident(\"url\"), got: {args:?}"
    );
}

#[test]
fn call_args_numeric_literal() {
    let src = r#"
class Caller {
    void run(cb) { schedule(cb, 1000) }
}
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::Literal(s) if s == "1000")),
        "expected Literal(\"1000\"), got: {args:?}"
    );
}

#[test]
fn call_args_ternary_expression_produces_ternary_variant() {
    let src = r#"
class Caller {
    void run(a, b, c) { f(a ? b : c) }
}
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter().any(|a| matches!(a, CallArg::Ternary { .. })),
        "expected Ternary variant for ternary arg, got: {args:?}"
    );
}

#[test]
fn call_args_array_literal_produces_array_literal_variant() {
    let src = r#"
class Caller {
    void run(x, y) { f([x, y]) }
}
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::ArrayLiteral { .. })),
        "expected ArrayLiteral variant for array arg, got: {args:?}"
    );
}

#[test]
fn call_args_array_access_produces_index_access_variant() {
    let src = r#"
class Caller {
    void run(a, i) { f(a[i]) }
}
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::IndexAccess { .. })),
        "expected IndexAccess variant for subscript arg, got: {args:?}"
    );
}

#[test]
fn call_args_binary_expression_produces_binary_variant() {
    let src = r#"
class Caller {
    void run(a, b) { f(a + b) }
}
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::Binary { op, .. } if op == "+")),
        "expected Binary variant with op \"+\" for addition arg, got: {args:?}"
    );
}

#[test]
fn object_creation_emits_instantiates_ref() {
    let src = r#"
class Caller {
    void run() {
        def analyzer = new Analyzer(new Source("text"))
    }
}
"#;
    let refs = extract::extract(src).refs;
    let names: Vec<&str> = refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Instantiates)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        names.contains(&"Analyzer"),
        "expected Instantiates for outer `new Analyzer`, got: {names:?}"
    );
    assert!(
        names.contains(&"Source"),
        "expected Instantiates for nested `new Source`, got: {names:?}"
    );
}

#[test]
fn object_creation_strips_qualifier_and_generics() {
    let src = r#"
class Caller {
    void run() {
        def list = new java.util.ArrayList<String>()
    }
}
"#;
    let refs = extract::extract(src).refs;
    assert!(
        refs.iter()
            .any(|r| r.kind == EdgeKind::Instantiates && r.target_name == "ArrayList"),
        "expected Instantiates target `ArrayList` (qualifier + generics stripped), got: {:?}",
        refs.iter()
            .filter(|r| r.kind == EdgeKind::Instantiates)
            .map(|r| &r.target_name)
            .collect::<Vec<_>>()
    );
}
