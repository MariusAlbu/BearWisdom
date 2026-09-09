use super::*;

#[test]
fn source_value_argument_serialization_keeps_expression_addresses_distinct_from_identifier_leaves()
{
    let value = CallArg::ValueAt(SourceSpan { start: 12, end: 28 });
    let json = serde_json::to_string(&value).unwrap();
    assert_eq!(json, r#"{"ValueAt":{"start":12,"end":28}}"#);
    assert_eq!(serde_json::from_str::<CallArg>(&json).unwrap(), value);
    value.visit_identifiers(&mut |_| panic!("a whole expression is not an identifier leaf"));
}

#[test]
fn borrowed_argument_roundtrip_retains_nested_source_identity_without_spelling() {
    let leaf = SourceSpan { start: 14, end: 15 };
    let arg = CallArg::BorrowAt {
        span: SourceSpan { start: 10, end: 15 },
        expr: Box::new(CallArg::BorrowAt {
            span: SourceSpan { start: 11, end: 15 },
            expr: Box::new(CallArg::IdentAt(leaf)),
        }),
    };
    let encoded = serde_json::to_string(&arg).unwrap();
    assert_eq!(serde_json::from_str::<CallArg>(&encoded).unwrap(), arg);
    let mut leaves = Vec::new();
    arg.visit_identifiers(&mut |span| leaves.push(span));
    assert_eq!(leaves, [leaf]);
}

#[test]
fn source_addressed_identifier_round_trips_without_a_name_payload() {
    let arg = CallArg::IdentAt(SourceSpan { start: 12, end: 17 });
    let payload = serde_json::to_string(&arg).unwrap();
    assert_eq!(payload, r#"{"IdentAt":{"start":12,"end":17}}"#);
    assert_eq!(serde_json::from_str::<CallArg>(&payload).unwrap(), arg);
    let legacy: CallArg = serde_json::from_str(r#"{"Ident":"legacy"}"#).unwrap();
    assert_eq!(legacy, CallArg::Ident("legacy".into()));
}

#[test]
fn nested_argument_traversal_visits_only_identifier_use_spans() {
    let first = SourceSpan { start: 10, end: 12 };
    let second = SourceSpan { start: 30, end: 31 };
    let arg = CallArg::Ternary {
        then_branch: Box::new(CallArg::ArrayLiteral {
            elements: vec![CallArg::IdentAt(first)],
        }),
        else_branch: Box::new(CallArg::Await {
            expr: Box::new(CallArg::IdentAt(second)),
        }),
    };
    let mut spans = Vec::new();
    arg.visit_identifiers(&mut |span| spans.push(span));
    assert_eq!(spans, vec![first, second]);
    CallArg::LambdaAt {
        params: vec![Some(first)],
    }
    .visit_identifiers(&mut |span| spans.push(span));
    assert_eq!(
        spans.len(),
        2,
        "callback declarations are not argument reads"
    );
}
