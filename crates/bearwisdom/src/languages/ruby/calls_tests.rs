// Tests for calls.rs — extract_call_args recursive variant extraction.

use crate::types::{CallArg, SourceSpan};

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

fn call_args_for(src: &str, target: &str) -> Vec<CallArg> {
    use crate::languages::ruby::extract;

    extract::extract(src)
        .refs
        .into_iter()
        .find(|reference| {
            reference.kind == crate::types::EdgeKind::Calls && reference.target_name == target
        })
        .map(|reference| reference.call_args)
        .unwrap_or_default()
}

fn lambda_param_slices<'a>(src: &'a str, args: &[CallArg]) -> Vec<Vec<Option<&'a str>>> {
    args.iter()
        .filter_map(|arg| match arg {
            CallArg::LambdaAt { params } => Some(
                params
                    .iter()
                    .map(|span| span.map(|span| &src[span.start as usize..span.end as usize]))
                    .collect(),
            ),
            _ => None,
        })
        .collect()
}

fn first_lambda_params(args: &[CallArg]) -> &[Option<SourceSpan>] {
    args.iter()
        .find_map(|arg| match arg {
            CallArg::LambdaAt { params } => Some(params.as_slice()),
            _ => None,
        })
        .expect("expected LambdaAt callback argument")
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
        args.iter()
            .any(|a| matches!(a, CallArg::StringLit(s) if s == "/api/users")),
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
        args.iter()
            .any(|a| matches!(a, CallArg::Ident(s) if s == "url")),
        "expected Ident(\"url\"), got: {args:?}"
    );
}

#[test]
fn call_args_brace_block_captures_exact_lambda_parameter_spans() {
    let src = r#"
def caller(arr)
  arr.map { |x| x.foo }
end
"#;
    let args = parse_call_args(src);
    assert_eq!(
        lambda_param_slices(src, &args),
        vec![vec![Some("x")]],
        "brace callback parameter must retain its exact declaration span: {args:?}"
    );
    assert_eq!(
        first_lambda_params(&args)[0].unwrap().start as usize,
        src.find("|x|").unwrap() + 1,
        "the parameter span must point at the declaration, not x.foo"
    );
}

#[test]
fn call_args_do_block_captures_exact_lambda_parameter_spans() {
    let src = r#"
def caller(arr)
  arr.each do |y|
    y.bar
  end
end
"#;
    let args = parse_call_args(src);
    assert_eq!(
        lambda_param_slices(src, &args),
        vec![vec![Some("y")]],
        "do callback parameter must retain its exact declaration span: {args:?}"
    );
    assert_eq!(
        first_lambda_params(&args)[0].unwrap().start as usize,
        src.find("|y|").unwrap() + 1,
        "the parameter span must point at the declaration, not y.bar"
    );
}

#[test]
fn call_args_direct_arrow_lambda_captures_ordered_declaration_spans() {
    let src = r#"
def caller
  consume(->(item, other) { item.touch(other) })
end
"#;
    let args = call_args_for(src, "consume");
    assert_eq!(
        lambda_param_slices(src, &args),
        vec![vec![Some("item"), Some("other")]],
        "direct arrow lambda parameters must remain positional: {args:?}"
    );
    assert_eq!(
        first_lambda_params(&args)[0].unwrap().start as usize,
        src.find("item, other").unwrap(),
        "the arrow parameter span must point at its declaration"
    );
}

#[test]
fn call_args_callback_factories_capture_only_exact_proc_shapes() {
    for (factory, parameter) in [
        ("proc { |item| item.touch }", "item"),
        ("lambda { |value| value.touch }", "value"),
        ("Proc.new { |entry| entry.touch }", "entry"),
    ] {
        let src = format!("def caller\n  consume({factory})\nend\n");
        let args = call_args_for(&src, "consume");
        assert_eq!(
            lambda_param_slices(&src, &args),
            vec![vec![Some(parameter)]],
            "factory `{factory}` must be a callback argument: {args:?}"
        );
    }

    let src = r#"
def caller(builder)
  consume(builder.proc { |not_a_callback| not_a_callback.touch })
end
"#;
    let args = call_args_for(src, "consume");
    assert!(
        lambda_param_slices(src, &args).is_empty(),
        "a receiver-owned proc method is not the Kernel proc factory: {args:?}"
    );
}

#[test]
fn call_args_zero_arity_callbacks_remain_explicit_lambda_arguments() {
    for source in [
        "def caller(arr)\n  arr.each { touch }\nend\n",
        "def caller(arr)\n  arr.each do\n    touch\n  end\nend\n",
    ] {
        let args = call_args_for(source, "each");
        assert_eq!(
            lambda_param_slices(source, &args),
            vec![Vec::new()],
            "zero-parameter trailing block must remain a callback argument: {args:?}"
        );
    }

    let arrow = "def caller\n  consume(-> { touch })\nend\n";
    assert_eq!(
        lambda_param_slices(arrow, &call_args_for(arrow, "consume")),
        vec![Vec::new()],
        "zero-parameter arrow must remain a callback argument"
    );

    for factory in ["proc { touch }", "lambda { touch }", "Proc.new { touch }"] {
        let source = format!("def caller\n  consume({factory})\nend\n");
        assert_eq!(
            lambda_param_slices(&source, &call_args_for(&source, "consume")),
            vec![Vec::new()],
            "zero-parameter `{factory}` must remain a callback argument"
        );
    }
}

#[test]
fn call_args_unsupported_callback_parameters_keep_holes_and_exclude_block_locals() {
    let src = r#"
def caller(arr, default)
  arr.map { |first, (left, right), *rest, keyword:, optional = default, &block; local| first.touch }
end
"#;
    let args = parse_call_args(src);
    assert_eq!(
        lambda_param_slices(src, &args),
        vec![vec![Some("first"), None, None, None, None, None]],
        "destructure/rest/keyword/optional/block forms need holes; block locals are not callback parameters: {args:?}"
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
        args.iter()
            .any(|a| matches!(a, CallArg::ArrayLiteral { .. })),
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
        args.iter()
            .any(|a| matches!(a, CallArg::IndexAccess { .. })),
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
        args.iter()
            .any(|a| matches!(a, CallArg::Binary { op, .. } if op == "+")),
        "expected Binary variant with op \"+\" for addition arg, got: {args:?}"
    );
}
