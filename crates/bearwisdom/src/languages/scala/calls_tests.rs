// Tests for calls.rs — lambda parameter-name capture in call args, including
// the brace-block call form whose args node is a `block`.

use crate::types::CallArg;

/// Parse a Scala snippet and return the args of the first `Calls` ref that
/// carries a captured lambda argument. Filtering for a `Lambda` arg targets the
/// higher-order call specifically — an inner call (`f(a, b)`) in the lambda body
/// carries `Ident` args, not a `Lambda`, so it is skipped.
fn parse_lambda_call_args(source: &str) -> Vec<CallArg> {
    let result = super::super::extract::extract(source);
    result
        .refs
        .into_iter()
        .find(|r| {
            r.kind == crate::types::EdgeKind::Calls
                && r.call_args
                    .iter()
                    .any(|a| matches!(a, CallArg::Lambda { .. }))
        })
        .map(|r| r.call_args)
        .unwrap_or_default()
}

#[test]
fn call_args_brace_block_lambda_param_captured() {
    // `list.map { x => x.foo }` — the args node is a `block`, not `arguments`.
    let src = r#"
object O { def caller(list: List[User]) = list.map { x => x.foo } }
"#;
    let args = parse_lambda_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::Lambda { params } if params.as_slice() == ["x"])),
        "expected Lambda {{ params: [\"x\"] }}, got: {args:?}"
    );
}

#[test]
fn call_args_brace_block_multi_param_lambda_captured() {
    // `{ (a, b) => f(a, b) }` — multi-param brace-block lambda.
    let src = r#"
object O { def caller(list: List[User]) = list.foldLeft(z) { (a, b) => f(a, b) } }
"#;
    let args = parse_lambda_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::Lambda { params } if params.as_slice() == ["a", "b"])),
        "expected Lambda {{ params: [\"a\", \"b\"] }}, got: {args:?}"
    );
}

#[test]
fn call_args_paren_arguments_lambda_still_captured() {
    // `list.map(x => x.foo)` — the parenthesized `arguments` path is unchanged.
    let src = r#"
object O { def caller(list: List[User]) = list.map(x => x.foo) }
"#;
    let args = parse_lambda_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::Lambda { params } if params.as_slice() == ["x"])),
        "expected Lambda {{ params: [\"x\"] }}, got: {args:?}"
    );
}

#[test]
fn call_args_non_lambda_block_contributes_nothing() {
    // `{ val t = x; t.foo }` — a multi-statement block with no lambda; the call
    // must not yield a `CallArg::Lambda`.
    let src = r#"
object O { def caller(x: User) = wrap { val t = x; t.foo } }
"#;
    let lambda_args = parse_lambda_call_args(src);
    assert!(
        lambda_args.is_empty(),
        "expected no Lambda from a non-lambda block, got: {lambda_args:?}"
    );
}

#[test]
fn type_arg_fqn_emits_trailing_name_only() {
    // A fully-qualified type argument (`stable_type_identifier`) must emit a
    // TypeRef to the trailing simple name only — never a bare package segment.
    let src = r#"
object O { val items: List[org.apache.commons.io.IOUtils] = null }
"#;
    let r = super::super::extract::extract(src);
    let type_refs: Vec<&str> = r
        .refs
        .iter()
        .filter(|rf| rf.kind == crate::types::EdgeKind::TypeRef)
        .map(|rf| rf.target_name.as_str())
        .collect();
    assert!(
        type_refs.contains(&"IOUtils"),
        "expected TypeRef to trailing name IOUtils; got {type_refs:?}"
    );
    for seg in ["org", "apache", "commons", "io"] {
        assert!(
            !type_refs.contains(&seg),
            "package segment {seg:?} leaked as a TypeRef; got {type_refs:?}"
        );
    }
}
