use super::*;
use serde_json::json;

#[test]
fn effective_iterator_policy_preserves_overrides_and_missing_evidence() {
    for (options, expected) in [
        (json!({}), Some(false)),
        (json!({"strict": true}), Some(true)),
        (
            json!({"strict": false, "strictBuiltinIteratorReturn": true}),
            Some(true),
        ),
        (
            json!({"strict": true, "strictBuiltinIteratorReturn": false}),
            Some(false),
        ),
        (
            json!({"strict": true, "strictNullChecks": false}),
            Some(true),
        ),
        (json!(null), None),
        (json!([]), None),
        (json!({"strict": "true"}), None),
        (json!({"strictBuiltinIteratorReturn": null}), None),
        (json!({"strictBuiltinIteratorReturn": 1}), None),
    ] {
        assert_eq!(
            typescript_options(&options).map(|p| p.strict_iterator_return),
            expected,
            "{options}"
        );
    }
}
