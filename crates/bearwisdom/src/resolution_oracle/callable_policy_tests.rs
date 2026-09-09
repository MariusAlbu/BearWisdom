use super::*;
use serde_json::json;

#[test]
fn explicit_options_override_strict_and_malformed_options_are_unknown() {
    assert_eq!(
        typescript_options(&json!({})),
        Some(CallablePolicy {
            strict_parameters: false,
            strict_nulls: false,
            bivariant_methods: Some(true)
        })
    );
    assert_eq!(
        typescript_options(&json!({"strict":true})),
        Some(CallablePolicy {
            strict_parameters: true,
            strict_nulls: true,
            bivariant_methods: Some(true)
        })
    );
    assert_eq!(
        typescript_options(&json!({"strict":true,"strictFunctionTypes":false})),
        Some(CallablePolicy {
            strict_parameters: false,
            strict_nulls: true,
            bivariant_methods: Some(true)
        })
    );
    assert_eq!(
        typescript_options(&json!({"strict":false,"strictNullChecks":true})),
        Some(CallablePolicy {
            strict_parameters: false,
            strict_nulls: true,
            bivariant_methods: Some(true)
        })
    );
    assert_eq!(
        typescript_options(&json!({"strictFunctionTypes":"false"})),
        None
    );
    assert_eq!(typescript_options(&json!(null)), None);
}
