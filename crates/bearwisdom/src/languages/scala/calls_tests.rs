// Tests for calls.rs — lambda parameter-span capture in call args, including
// the brace-block call form whose args node is a `block`.

use crate::types::{CallArg, EdgeKind, SegmentKind};

/// Parse a Scala snippet and return the args of the first `Calls` ref that
/// carries a captured lambda argument. Filtering for a `LambdaAt` arg targets the
/// higher-order call specifically — an inner call (`f(a, b)`) in the lambda body
/// carries `Ident` args, not a callback argument, so it is skipped.
fn parse_lambda_call_args(source: &str) -> Vec<CallArg> {
    let result = super::super::extract::extract(source);
    result
        .refs
        .into_iter()
        .find(|r| {
            r.kind == crate::types::EdgeKind::Calls
                && r.call_args
                    .iter()
                    .any(|a| matches!(a, CallArg::LambdaAt { .. }))
        })
        .map(|r| r.call_args)
        .unwrap_or_default()
}

fn callback_parameters<'a>(source: &'a str, args: &[CallArg]) -> Vec<Vec<Option<&'a str>>> {
    args.iter()
        .filter_map(|arg| match arg {
            CallArg::LambdaAt { params } => Some(
                params
                    .iter()
                    .map(|span| span.map(|s| &source[s.start as usize..s.end as usize]))
                    .collect(),
            ),
            _ => None,
        })
        .collect()
}

#[test]
fn call_args_brace_block_lambda_param_captured() {
    // `list.map { x => x.foo }` — the args node is a `block`, not `arguments`.
    let src = r#"
object O { def caller(list: List[User]) = list.map { x => x.foo } }
"#;
    let args = parse_lambda_call_args(src);
    assert_eq!(callback_parameters(src, &args), vec![vec![Some("x")]]);
}

#[test]
fn call_args_brace_block_multi_param_lambda_captured() {
    // `{ (a, b) => f(a, b) }` — multi-param brace-block lambda.
    let src = r#"
object O { def caller(list: List[User]) = list.foldLeft(z) { (a, b) => f(a, b) } }
"#;
    let args = parse_lambda_call_args(src);
    assert_eq!(
        callback_parameters(src, &args),
        vec![vec![Some("a"), Some("b")]]
    );
}

#[test]
fn call_args_paren_arguments_lambda_still_captured() {
    // `list.map(x => x.foo)` — the parenthesized `arguments` path is unchanged.
    let src = r#"
object O { def caller(list: List[User]) = list.map(x => x.foo) }
"#;
    let args = parse_lambda_call_args(src);
    assert_eq!(callback_parameters(src, &args), vec![vec![Some("x")]]);
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

#[test]
fn this_member_call_emits_self_root_and_called_property() {
    let result = super::super::extract::extract(
        "class Same { def modify() = (); def call() = this.modify() }",
    );
    let calls: Vec<_> = result
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Calls)
        .collect();
    assert_eq!(calls.len(), 1, "expected one this.modify call: {calls:?}");

    let segments = &calls[0]
        .chain
        .as_ref()
        .expect("this.modify must carry a member chain")
        .segments;
    assert_eq!(segments.len(), 2, "unexpected chain: {segments:?}");
    assert_eq!(segments[0].name, "this");
    assert_eq!(segments[0].kind, SegmentKind::SelfRef);
    assert_eq!(segments[1].name, "modify");
    assert_eq!(segments[1].kind, SegmentKind::Property);
    assert!(segments[1].is_call, "modify must be marked as called");
}
