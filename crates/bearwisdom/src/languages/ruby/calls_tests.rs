// Tests for calls.rs — extract_call_args recursive variant extraction.

use crate::types::CallArg;

/// Parse a Ruby snippet and return the `call_args` of the first `Calls` ref
/// that carries arguments.
fn parse_call_args(src: &str) -> Vec<CallArg> {
    use crate::languages::ruby::extract;

    let result = extract::extract(src);
    result
        .refs
        .into_iter()
        .find(|r| r.kind == crate::types::EdgeKind::Calls && !r.call_args.is_empty())
        .map(|r| r.call_args)
        .unwrap_or_default()
}

#[test]
fn call_args_string_literal_preserved() {
    let src = r#"
def caller
  fetch("/api/users")
end
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter().any(|a| matches!(a, CallArg::StringLit(s) if s == "/api/users")),
        "expected StringLit(\"/api/users\"), got: {args:?}"
    );
}

#[test]
fn call_args_identifier_becomes_ident_variant() {
    let src = r#"
def caller(url)
  fetch(url)
end
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter().any(|a| matches!(a, CallArg::Ident(s) if s == "url")),
        "expected Ident(\"url\"), got: {args:?}"
    );
}

#[test]
fn call_args_conditional_produces_ternary_variant() {
    let src = r#"
def caller(a, b, c)
  f(a ? b : c)
end
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter().any(|a| matches!(a, CallArg::Ternary { .. })),
        "expected Ternary variant for conditional arg, got: {args:?}"
    );
}

#[test]
fn call_args_array_literal_produces_array_literal_variant() {
    let src = r#"
def caller(x, y)
  f([x, y])
end
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter().any(|a| matches!(a, CallArg::ArrayLiteral { .. })),
        "expected ArrayLiteral variant for array arg, got: {args:?}"
    );
}

#[test]
fn call_args_splat_produces_spread_variant() {
    let src = r#"
def caller(xs)
  f(*xs)
end
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter().any(|a| matches!(a, CallArg::Spread { .. })),
        "expected Spread variant for splat arg, got: {args:?}"
    );
}

#[test]
fn call_args_element_reference_produces_index_access_variant() {
    let src = r#"
def caller(a, i)
  f(a[i])
end
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter().any(|a| matches!(a, CallArg::IndexAccess { .. })),
        "expected IndexAccess variant for element_reference arg, got: {args:?}"
    );
}

#[test]
fn call_args_binary_produces_binary_variant() {
    let src = r#"
def caller(a, b)
  f(a + b)
end
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter().any(|a| matches!(a, CallArg::Binary { op, .. } if op == "+")),
        "expected Binary variant with op \"+\" for addition arg, got: {args:?}"
    );
}
