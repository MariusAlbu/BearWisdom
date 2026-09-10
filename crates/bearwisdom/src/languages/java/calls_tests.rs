// Tests for calls.rs — extract_call_args recursive CallArg variants.

use super::extract;
use crate::types::{CallArg, EdgeKind};

/// Parse a Java source snippet and return the `call_args` of the first
/// `Calls` ref whose target matches `target` and that carries arguments.
fn call_args_for(src: &str, target: &str) -> Vec<CallArg> {
    extract::extract(src)
        .refs
        .into_iter()
        .find(|r| r.kind == EdgeKind::Calls && r.target_name == target && !r.call_args.is_empty())
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
fn call_args_lambda_parameters_use_exact_declaration_spans() {
    let src = r#"
class C {
    void m() {
        bare(x -> x.run());
        inferred((left, right) -> left.run());
        formal((String name, int count) -> name.trim());
    }
}
"#;
    assert_eq!(
        callback_parameters(src, &call_args_for(src, "bare")),
        vec![vec![Some("x")]]
    );
    assert_eq!(
        callback_parameters(src, &call_args_for(src, "inferred")),
        vec![vec![Some("left"), Some("right")]]
    );
    assert_eq!(
        callback_parameters(src, &call_args_for(src, "formal")),
        vec![vec![Some("name"), Some("count")]]
    );
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
        args.iter()
            .any(|a| matches!(a, CallArg::StringLit(s) if s == "/api/users")),
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
        args.iter()
            .any(|a| matches!(a, CallArg::Ident(s) if s == "url")),
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
        args.iter()
            .any(|a| matches!(a, CallArg::ArrayLiteral { .. })),
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
        args.iter()
            .any(|a| matches!(a, CallArg::IndexAccess { .. })),
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
        args.iter()
            .any(|a| matches!(a, CallArg::Binary { op, .. } if op == "+")),
        "expected Binary variant with op \"+\", got: {args:?}"
    );
}

// -----------------------------------------------------------------------
// Method-invocation receivers: a bare receiver (`X.y()`) is carried on the
// Calls ref's chain root, never duplicated as a TypeRef. A redundant TypeRef
// to the receiver is permanently dead when the receiver is a field.
// -----------------------------------------------------------------------

/// All refs of `kind` whose target is `target`.
fn refs_to(src: &str, kind: EdgeKind, target: &str) -> Vec<crate::types::ExtractedRef> {
    extract::extract(src)
        .refs
        .into_iter()
        .filter(|r| r.kind == kind && r.target_name == target)
        .collect()
}

#[test]
fn static_field_read_receiver_emits_no_type_ref() {
    let src = r#"
class Service {
    private static final Logger LOG = LoggerFactory.getLogger(Service.class);
    void run() {
        LOG.debug("x");
    }
}
"#;
    assert!(
        refs_to(src, EdgeKind::TypeRef, "LOG").is_empty(),
        "no TypeRef to the field receiver LOG should be emitted"
    );
    // The receiver is preserved as the Calls ref's chain root.
    let call = refs_to(src, EdgeKind::Calls, "debug");
    assert_eq!(call.len(), 1, "expected a single Calls ref to debug");
    let chain = call[0].chain.as_ref().expect("debug call carries a chain");
    assert_eq!(
        chain.segments.first().map(|s| s.name.as_str()),
        Some("LOG"),
        "chain root must be the receiver LOG"
    );
}

#[test]
fn static_method_call_receiver_resolves_via_chain() {
    let src = r#"
import java.util.Collections;
import java.util.List;
class Service {
    void run(List<String> list) {
        Collections.sort(list);
    }
}
"#;
    // The static call is preserved as a Calls ref whose chain root names the
    // class — the chain walker resolves `Collections` from that root.
    let call = refs_to(src, EdgeKind::Calls, "sort");
    assert_eq!(call.len(), 1, "expected a single Calls ref to sort");
    let chain = call[0].chain.as_ref().expect("sort call carries a chain");
    assert_eq!(
        chain.segments.first().map(|s| s.name.as_str()),
        Some("Collections"),
        "chain root must be the receiver class Collections"
    );
    assert_eq!(chain.segments.last().map(|s| s.name.as_str()), Some("sort"));
    // No standalone TypeRef edge to the two-segment receiver.
    assert!(
        refs_to(src, EdgeKind::TypeRef, "Collections").is_empty(),
        "two-segment static-call receiver must not emit a redundant TypeRef"
    );
}

#[test]
fn nested_namespace_call_still_emits_intermediate_type_ref() {
    let src = r#"
class Service {
    void run() {
        Stripe.Event.create();
    }
}
"#;
    // A three-segment chain carries a genuine intermediate type before the
    // method — that prefix TypeRef is preserved.
    assert!(
        !refs_to(src, EdgeKind::TypeRef, "Event").is_empty(),
        "intermediate namespace type Event should still emit a TypeRef"
    );
}
