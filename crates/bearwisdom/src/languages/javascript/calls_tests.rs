// Tests for calls.rs — call-argument extraction via the shared
// `languages::common::extract_call_args` helper.

use crate::languages::javascript::extract;
use crate::types::{CallArg, EdgeKind};

/// Parse a JavaScript snippet and return the `call_args` of the first Calls
/// ref whose target is `callee`.
fn call_args_for(src: &str, callee: &str) -> Vec<CallArg> {
    let result = extract::extract(src);
    result
        .refs
        .into_iter()
        .find(|r| r.kind == EdgeKind::Calls && r.target_name == callee)
        .map(|r| r.call_args)
        .unwrap_or_default()
}

#[test]
fn lambda_arg_single_param_arrow() {
    // `arr.map(x => x.foo)` — the map call carries a Lambda arg whose sole
    // positional parameter is `x`.
    let src = r#"
function caller() { arr.map(x => x.foo); }
"#;
    let args = call_args_for(src, "map");
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::Lambda { params } if params == &vec!["x".to_string()])),
        "expected CallArg::Lambda {{ params: [\"x\"] }}, got: {args:?}"
    );
}

#[test]
fn lambda_arg_function_expression() {
    // `arr.forEach(function (item) { ... })` — function-expression argument
    // carries its parameter name.
    let src = r#"
function caller() { arr.forEach(function (item) { use(item); }); }
"#;
    let args = call_args_for(src, "forEach");
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::Lambda { params } if params == &vec!["item".to_string()])),
        "expected CallArg::Lambda {{ params: [\"item\"] }}, got: {args:?}"
    );
}

#[test]
fn lambda_arg_multi_param_parenthesized_arrow() {
    // `arr.reduce((acc, cur) => acc + cur)` — both parameters captured in order.
    let src = r#"
function caller() { arr.reduce((acc, cur) => acc + cur); }
"#;
    let args = call_args_for(src, "reduce");
    assert!(
        args.iter().any(|a| matches!(
            a,
            CallArg::Lambda { params } if params == &vec!["acc".to_string(), "cur".to_string()]
        )),
        "expected CallArg::Lambda {{ params: [\"acc\", \"cur\"] }}, got: {args:?}"
    );
}

#[test]
fn string_literal_arg_captured() {
    // The shared helper also lands plain string args, proving JS now populates
    // call_args at all (it built `Vec::new()` before).
    let src = r#"
function caller() { fetch("/api/users"); }
"#;
    let args = call_args_for(src, "fetch");
    assert!(
        args.iter().any(|a| matches!(a, CallArg::StringLit(s) if s == "/api/users")),
        "expected StringLit(\"/api/users\"), got: {args:?}"
    );
}
