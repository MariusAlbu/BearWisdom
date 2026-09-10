// Tests for calls.rs — extract_call_args recursive variant extraction.

use super::extract;
use crate::types::CallArg;

/// Parse a C# snippet and return the `call_args` of the first `Calls` ref that
/// carries any arguments. The args travel via the `ExtractedRef::call_args`
/// the extractor stores, so this exercises the real tree-sitter path rather
/// than calling `extract_arg` against a synthetic node.
fn parse_call_args(source: &str) -> Vec<CallArg> {
    let result = extract::extract(source);
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
fn call_args_conditional_expression_produces_ternary_variant() {
    let src = r#"
class C { void M(bool cond, int a, int b) { F(cond ? a : b); } }
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter().any(|a| matches!(a, CallArg::Ternary { .. })),
        "expected Ternary variant for conditional arg, got: {args:?}"
    );
}

#[test]
fn call_args_array_creation_produces_array_literal_variant() {
    let src = r#"
class C { void M(int x, int y) { F(new int[] { x, y }); } }
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::ArrayLiteral { elements } if elements.len() == 2)),
        "expected ArrayLiteral with two elements, got: {args:?}"
    );
}

#[test]
fn call_args_implicit_array_creation_produces_array_literal_variant() {
    let src = r#"
class C { void M(int x, int y) { F(new[] { x, y }); } }
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::ArrayLiteral { elements } if elements.len() == 2)),
        "expected ArrayLiteral with two elements, got: {args:?}"
    );
}

#[test]
fn call_args_await_expression_produces_await_variant() {
    let src = r#"
class C { async Task M() { F(await GetValue()); } }
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter().any(|a| matches!(a, CallArg::Await { .. })),
        "expected Await variant for awaited arg, got: {args:?}"
    );
}

#[test]
fn call_args_element_access_produces_index_access_variant() {
    let src = r#"
class C { void M(int[] arr, int i) { F(arr[i]); } }
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter().any(|a| matches!(
            a,
            CallArg::IndexAccess { container, index }
                if matches!(**container, CallArg::Ident(ref s) if s == "arr")
                    && matches!(**index, CallArg::Ident(ref s) if s == "i")
        )),
        "expected IndexAccess(arr, i), got: {args:?}"
    );
}

#[test]
fn call_args_binary_expression_produces_binary_variant() {
    let src = r#"
class C { void M(int a, int b) { F(a + b); } }
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::Binary { op, .. } if op == "+")),
        "expected Binary variant with `+` operator, got: {args:?}"
    );
}

#[test]
fn call_args_simple_variants_unchanged() {
    // The existing simple-variant behaviour must survive the refactor.
    let src = r#"
class C { void M(string url) { F("/api/users", url, 42, true); } }
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::StringLit(s) if s == "/api/users")),
        "expected StringLit(\"/api/users\"), got: {args:?}"
    );
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::Ident(s) if s == "url")),
        "expected Ident(\"url\"), got: {args:?}"
    );
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::Literal(s) if s == "42")),
        "expected Literal(\"42\"), got: {args:?}"
    );
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::Literal(s) if s == "true")),
        "expected Literal(\"true\"), got: {args:?}"
    );
}

#[test]
fn call_args_lambda_implicit_single_param_captured() {
    // `u => u.Name` — single implicit parameter.
    let src = r#"
class C { void M(System.Collections.Generic.List<int> users) { users.Select(u => u.Name); } }
"#;
    let args = parse_call_args(src);
    assert_eq!(callback_parameters(src, &args), vec![vec![Some("u")]]);
}

#[test]
fn call_args_lambda_parenthesized_params_captured() {
    // `(x, y) => f(x, y)` — parenthesized parameter list.
    let src = r#"
class C { void M(System.Collections.Generic.List<int> xs) { xs.Select((x, y) => f(x, y)); } }
"#;
    let args = parse_call_args(src);
    assert_eq!(
        callback_parameters(src, &args),
        vec![vec![Some("x"), Some("y")]]
    );
}

#[test]
fn named_argument_unwraps_to_the_value_expression() {
    // A named argument (`columns: table => ...`) wraps the value behind a
    // name-colon node; the capture must skip the name and take the VALUE, or
    // the lambda degrades to Other and its params never seed.
    let src = r#"
class C { void M() { F(name: "Events", columns: table => table); } }
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::StringLit(s) if s == "Events")),
        "named string arg must capture its value, got: {args:?}"
    );
    assert_eq!(callback_parameters(src, &args), vec![vec![Some("table")]]);
}

#[test]
fn call_args_explicit_and_anonymous_method_parameters_use_declaration_spans() {
    let src = r#"
class C {
    void M() {
        F((string name, int count) => name.Trim());
        F(delegate(string item) { item.ToString(); });
    }
}
"#;
    let result = extract::extract(src);
    let callbacks: Vec<_> = result
        .refs
        .iter()
        .filter(|r| r.kind == crate::types::EdgeKind::Calls && r.target_name == "F")
        .flat_map(|r| callback_parameters(src, &r.call_args))
        .collect();
    assert_eq!(
        callbacks,
        vec![vec![Some("name"), Some("count")], vec![Some("item")]]
    );
}

#[test]
fn predefined_type_receiver_roots_the_chain_on_its_bcl_type() {
    let src = r#"class C { void M(string x) { string.IsNullOrWhiteSpace(x); } }"#;
    let result = extract::extract(src);
    let r = result
        .refs
        .iter()
        .find(|r| r.target_name == "IsNullOrWhiteSpace")
        .expect("call ref");
    let chain = r
        .chain
        .as_ref()
        .expect("chain must survive a keyword receiver");
    assert_eq!(chain.segments[0].name, "string");
    assert_eq!(
        chain.segments[0].declared_type.as_deref(),
        Some("System.String")
    );
}
