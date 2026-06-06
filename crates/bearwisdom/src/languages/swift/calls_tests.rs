// Tests for calls.rs — closure parameter-name capture in call args.

use crate::types::CallArg;

/// Parse a Swift snippet and return the args of the first `Calls` ref that
/// carries any captured arguments.
fn parse_call_args(source: &str) -> Vec<CallArg> {
    let result = super::super::extract::extract(source);
    result
        .refs
        .into_iter()
        .find(|r| r.kind == crate::types::EdgeKind::Calls && !r.call_args.is_empty())
        .map(|r| r.call_args)
        .unwrap_or_default()
}

#[test]
fn call_args_named_closure_param_captured() {
    // `{ x in x.foo }` — named parameter; the shorthand path must not fire.
    let src = r#"
func caller(arr: [User]) { arr.map { x in x.foo } }
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::Lambda { params } if params.as_slice() == ["x"])),
        "expected Lambda {{ params: [\"x\"] }}, got: {args:?}"
    );
}

#[test]
fn call_args_anonymous_shorthand_closure_dollar_params() {
    // `{ $0.foo }` — anonymous-shorthand closure declares `$0` only.
    let src = r#"
func caller(arr: [User]) { arr.map { $0.foo } }
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::Lambda { params } if params.as_slice() == ["$0"])),
        "expected Lambda {{ params: [\"$0\"] }}, got: {args:?}"
    );
}

#[test]
fn call_args_multi_shorthand_closure_dense_params() {
    // `{ $0.a + $1.b }` — highest index is `$1`, so the dense list is `$0,$1`.
    let src = r#"
func caller(arr: [User]) { arr.reduce { $0.a + $1.b } }
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::Lambda { params } if params.as_slice() == ["$0", "$1"])),
        "expected Lambda {{ params: [\"$0\", \"$1\"] }}, got: {args:?}"
    );
}

#[test]
fn call_args_shorthand_gap_emits_dense_list() {
    // `{ $1.y }` — only `$1` is referenced; the dense list still includes the
    // dead `$0` slot (a harmless no-op seed).
    let src = r#"
func caller(arr: [User]) { arr.map { $1.y } }
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::Lambda { params } if params.as_slice() == ["$0", "$1"])),
        "expected Lambda {{ params: [\"$0\", \"$1\"] }}, got: {args:?}"
    );
}

#[test]
fn call_args_nested_shorthand_does_not_leak_to_outer() {
    // The outer closure references no `$N` of its own; the inner closure's `$0`
    // must not leak into the outer parameter list. Scope stops at the nested
    // `lambda_literal` boundary, so the outer list is empty.
    let src = r#"
func caller(arr: [User]) { arr.map { outer in inner.map { $0.foo } } }
"#;
    let args = parse_call_args(src);
    // The outer closure is named (`outer in`), so its params are `["outer"]`,
    // never `["$0"]` from the inner closure.
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::Lambda { params } if params.as_slice() == ["outer"])),
        "expected outer Lambda {{ params: [\"outer\"] }} (no inner $0 leak), got: {args:?}"
    );
}
