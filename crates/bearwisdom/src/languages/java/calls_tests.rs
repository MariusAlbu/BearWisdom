// Tests for calls.rs — extract_call_args recursive CallArg variants.

use super::extract;
use crate::types::{CallArg, EdgeKind};

/// Parse a Java source snippet and return the `call_args` of the first
/// `Calls` ref whose target matches `target` and that carries arguments.
fn call_args_for(src: &str, target: &str) -> Vec<CallArg> {
    extract::extract(src)
        .refs
        .into_iter()
        .find(|r| {
            r.kind == EdgeKind::Calls && r.target_name == target && !r.call_args.is_empty()
        })
        .map(|r| r.call_args)
        .unwrap_or_default()
}

#[test]
fn call_args_string_literal() {
    let src = r#"
class C {
    void m() { f("/api/users"); }
}
"#;
    let args = call_args_for(src, "f");
    assert!(
        args.iter().any(|a| matches!(a, CallArg::StringLit(s) if s == "/api/users")),
        "expected StringLit, got: {args:?}"
    );
}

#[test]
fn call_args_identifier_becomes_ident_variant() {
    let src = r#"
class C {
    void m(String url) { f(url); }
}
"#;
    let args = call_args_for(src, "f");
    assert!(
        args.iter().any(|a| matches!(a, CallArg::Ident(s) if s == "url")),
        "expected Ident(\"url\"), got: {args:?}"
    );
}

#[test]
fn call_args_ternary_expression_produces_ternary_variant() {
    let src = r#"
class C {
    void m(boolean cond, String a, String b) { f(cond ? a : b); }
}
"#;
    let args = call_args_for(src, "f");
    assert!(
        args.iter().any(|a| matches!(a, CallArg::Ternary { .. })),
        "expected Ternary variant for ternary arg, got: {args:?}"
    );
}

#[test]
fn call_args_array_initializer_produces_array_literal_variant() {
    let src = r#"
class C {
    void m(int x, int y) { f(new int[]{x, y}); }
}
"#;
    let args = call_args_for(src, "f");
    assert!(
        args.iter().any(|a| matches!(a, CallArg::ArrayLiteral { .. })),
        "expected ArrayLiteral variant for array arg, got: {args:?}"
    );
}

#[test]
fn call_args_array_access_produces_index_access_variant() {
    let src = r#"
class C {
    void m(int[] a, int i) { f(a[i]); }
}
"#;
    let args = call_args_for(src, "f");
    assert!(
        args.iter().any(|a| matches!(a, CallArg::IndexAccess { .. })),
        "expected IndexAccess variant for array-access arg, got: {args:?}"
    );
}

#[test]
fn call_args_binary_expression_produces_binary_variant() {
    let src = r#"
class C {
    void m(int a, int b) { f(a + b); }
}
"#;
    let args = call_args_for(src, "f");
    assert!(
        args.iter().any(|a| matches!(a, CallArg::Binary { op, .. } if op == "+")),
        "expected Binary variant with op \"+\", got: {args:?}"
    );
}
