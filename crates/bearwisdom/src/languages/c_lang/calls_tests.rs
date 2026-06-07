// =============================================================================
// c_lang/calls_tests.rs  —  Operator-call ref emission (C++ ADL)
// =============================================================================

use super::*;
use crate::types::EdgeKind;

/// Collect the target names of all `Calls` refs emitted for `src`.
fn call_targets(src: &str, language: &str) -> Vec<String> {
    extract::extract(src, language)
        .refs
        .into_iter()
        .filter(|r| r.kind == EdgeKind::Calls)
        .map(|r| r.target_name)
        .collect()
}

#[test]
fn cpp_binary_plus_emits_operator_call() {
    let src = r#"
int f(int a, int b) {
    return a + b;
}
"#;
    let targets = call_targets(src, "cpp");
    assert!(
        targets.iter().any(|t| t == "operator+"),
        "expected operator+ Calls ref, got {targets:?}"
    );
}

#[test]
fn cpp_shift_emits_operator_call() {
    let src = r#"
void log(Stream& os, int x) {
    os << x;
}
"#;
    let targets = call_targets(src, "cpp");
    assert!(
        targets.iter().any(|t| t == "operator<<"),
        "expected operator<< Calls ref, got {targets:?}"
    );
}

#[test]
fn cpp_equality_emits_operator_call() {
    let src = r#"
bool eq(Vec a, Vec b) {
    return a == b;
}
"#;
    let targets = call_targets(src, "cpp");
    assert!(
        targets.iter().any(|t| t == "operator=="),
        "expected operator== Calls ref, got {targets:?}"
    );
}

#[test]
fn cpp_subscript_emits_operator_call() {
    let src = r#"
int at(Container c, int i) {
    return c[i];
}
"#;
    let targets = call_targets(src, "cpp");
    assert!(
        targets.iter().any(|t| t == "operator[]"),
        "expected operator[] Calls ref, got {targets:?}"
    );
}

#[test]
fn c_does_not_emit_operator_calls() {
    // C has no operator overloading — the `+` and `[]` arms must stay silent.
    let src = r#"
int f(int a, int b, int* arr, int i) {
    return a + b + arr[i];
}
"#;
    let targets = call_targets(src, "c");
    assert!(
        !targets.iter().any(|t| t.starts_with("operator")),
        "C must not emit operator Calls refs, got {targets:?}"
    );
}
