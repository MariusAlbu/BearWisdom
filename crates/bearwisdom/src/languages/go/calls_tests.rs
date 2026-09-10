// =============================================================================
// go/calls_tests.rs  —  Recursive CallArg extraction for Go call arguments
//
// Each test parses a tiny Go snippet whose call passes one complex-shape
// argument and asserts the produced `CallArg` matches the expected variant.
// =============================================================================

use super::extract;
use crate::types::{CallArg, EdgeKind};

/// Return the `call_args` of the first `Calls` ref whose target matches `name`.
fn call_args_of(src: &str, name: &str) -> Vec<CallArg> {
    let r = extract::extract(src);
    r.refs
        .iter()
        .find(|rf| rf.kind == EdgeKind::Calls && rf.target_name == name)
        .map(|rf| rf.call_args.clone())
        .unwrap_or_else(|| {
            panic!(
                "no Calls ref to '{name}'; calls: {:?}",
                r.refs
                    .iter()
                    .filter(|rf| rf.kind == EdgeKind::Calls)
                    .map(|rf| &rf.target_name)
                    .collect::<Vec<_>>()
            )
        })
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
fn spread_argument_produces_spread_variant() {
    // `f(items...)` — variadic spread of a slice argument.
    let src = "package main\nfunc f(xs ...int) {}\nfunc g(items []int) { f(items...) }";
    let args = call_args_of(src, "f");
    assert_eq!(args.len(), 1, "expected one arg; got {:?}", args);
    match &args[0] {
        CallArg::Spread { expr } => match expr.as_ref() {
            CallArg::Ident(name) => assert_eq!(name, "items"),
            other => panic!("expected Spread of Ident(items); got {:?}", other),
        },
        other => panic!("expected CallArg::Spread; got {:?}", other),
    }
}

#[test]
fn index_argument_produces_index_access_variant() {
    // `f(m["k"])` — subscript / index access argument.
    let src = "package main\nfunc f(v int) {}\nfunc g(m map[string]int) { f(m[\"k\"]) }";
    let args = call_args_of(src, "f");
    assert_eq!(args.len(), 1, "expected one arg; got {:?}", args);
    match &args[0] {
        CallArg::IndexAccess { container, index } => {
            match container.as_ref() {
                CallArg::Ident(name) => assert_eq!(name, "m"),
                other => panic!("expected container Ident(m); got {:?}", other),
            }
            match index.as_ref() {
                CallArg::StringLit(s) => assert_eq!(s, "k"),
                other => panic!("expected index StringLit(k); got {:?}", other),
            }
        }
        other => panic!("expected CallArg::IndexAccess; got {:?}", other),
    }
}

#[test]
fn binary_argument_produces_binary_variant() {
    // `f(a + b)` — binary expression argument.
    let src = "package main\nfunc f(v int) {}\nfunc g(a int, b int) { f(a + b) }";
    let args = call_args_of(src, "f");
    assert_eq!(args.len(), 1, "expected one arg; got {:?}", args);
    match &args[0] {
        CallArg::Binary { op, left, right } => {
            assert_eq!(op, "+");
            match left.as_ref() {
                CallArg::Ident(name) => assert_eq!(name, "a"),
                other => panic!("expected left Ident(a); got {:?}", other),
            }
            match right.as_ref() {
                CallArg::Ident(name) => assert_eq!(name, "b"),
                other => panic!("expected right Ident(b); got {:?}", other),
            }
        }
        other => panic!("expected CallArg::Binary; got {:?}", other),
    }
}

#[test]
fn string_literal_argument_preserved() {
    // Existing simple-variant behavior must be unchanged.
    let src = "package main\nimport \"fmt\"\nfunc g() { fmt.Println(\"hi\") }";
    let args = call_args_of(src, "Println");
    assert_eq!(args.len(), 1, "expected one arg; got {:?}", args);
    match &args[0] {
        CallArg::StringLit(s) => assert_eq!(s, "hi"),
        other => panic!("expected CallArg::StringLit; got {:?}", other),
    }
}

#[test]
fn function_literal_named_parameter_uses_its_exact_span() {
    let src = r#"package main
func Visit(fn func(*Item)) {}
func Run() { Visit(func(item *Item) { item.Touch() }) }
"#;

    assert_eq!(
        callback_parameters(src, &call_args_of(src, "Visit")),
        vec![vec![Some("item")]]
    );
}

#[test]
fn function_literal_grouped_parameters_expand_in_source_order() {
    let src = r#"package main
func Visit(fn func(Item, Item)) {}
func Run() { Visit(func(left, right Item) { left.Touch(); right.Touch() }) }
"#;

    assert_eq!(
        callback_parameters(src, &call_args_of(src, "Visit")),
        vec![vec![Some("left"), Some("right")]]
    );
}

#[test]
fn function_literal_unnamed_parameter_keeps_an_unbound_slot() {
    let src = r#"package main
func Visit(fn func(Item)) {}
func Run() { Visit(func(Item) {}) }
"#;

    assert_eq!(
        callback_parameters(src, &call_args_of(src, "Visit")),
        vec![vec![None]]
    );
}

#[test]
fn function_literal_blank_parameter_keeps_an_unbound_slot() {
    let src = r#"package main
func Visit(fn func(Item)) {}
func Run() { Visit(func(_ Item) {}) }
"#;

    assert_eq!(
        callback_parameters(src, &call_args_of(src, "Visit")),
        vec![vec![None]]
    );
}
