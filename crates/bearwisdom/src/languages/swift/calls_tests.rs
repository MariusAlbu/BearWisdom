// Tests for calls.rs — closure parameter-span capture in call args.

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
fn call_args_named_closure_params_use_exact_declaration_spans() {
    // `{ (left, right) in ... }` — names are declaration tokens, not uses in
    // the body. The source slices prove the stored spans address those tokens.
    let src = r#"
func caller(arr: [User]) { arr.reduce { (left, right) in left.foo(right) } }
"#;
    assert_eq!(
        callback_parameters(src, &parse_call_args(src)),
        vec![vec![Some("left"), Some("right")]]
    );
}

#[test]
fn call_args_anonymous_shorthand_closure_dollar_params() {
    // `{ $0.foo }` — `$0` is a use token, not a parameter declaration, so the
    // one callback position must remain an identity-free hole.
    let src = r#"
func caller(arr: [User]) { arr.map { $0.foo } }
"#;
    assert_eq!(
        callback_parameters(src, &parse_call_args(src)),
        vec![vec![None]]
    );
}

#[test]
fn call_args_wildcard_closure_parameter_keeps_a_positional_hole() {
    // `_` is syntactically present but creates no binding that a contextual type
    // could safely address.
    let src = r#"
func caller(arr: [User]) { arr.map { _ in "constant" } }
"#;
    assert_eq!(
        callback_parameters(src, &parse_call_args(src)),
        vec![vec![None]]
    );
}

#[test]
fn call_args_multi_shorthand_closure_dense_params() {
    // `{ $0.a + $1.b }` — highest index is `$1`, so both callback positions
    // remain holes even though their shorthand uses appear in the body.
    let src = r#"
func caller(arr: [User]) { arr.reduce { $0.a + $1.b } }
"#;
    assert_eq!(
        callback_parameters(src, &parse_call_args(src)),
        vec![vec![None, None]]
    );
}

#[test]
fn call_args_shorthand_gap_emits_dense_list() {
    // `{ $1.y }` — the unused first position remains present, and neither
    // position has a declaration-backed identity.
    let src = r#"
func caller(arr: [User]) { arr.map { $1.y } }
"#;
    assert_eq!(
        callback_parameters(src, &parse_call_args(src)),
        vec![vec![None, None]]
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
    // The outer closure's declaration span must survive; inner `$0` cannot
    // manufacture a source identity for it.
    assert_eq!(callback_parameters(src, &args), vec![vec![Some("outer")]]);
}
