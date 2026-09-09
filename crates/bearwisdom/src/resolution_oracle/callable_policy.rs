//! Compiler-option spelling is consumed only at the independently captured input boundary.
use crate::indexer::programs::CallablePolicy;

pub(super) fn typescript_options(options: &serde_json::Value) -> Option<CallablePolicy> {
    let object = options.as_object()?;
    let flag = |key| match object.get(key) {
        None => Some(None),
        Some(value) => value.as_bool().map(Some),
    };
    let strict = flag("strict")?.unwrap_or(false);
    Some(CallablePolicy {
        strict_parameters: flag("strictFunctionTypes")?.unwrap_or(strict),
        strict_nulls: flag("strictNullChecks")?.unwrap_or(strict),
        bivariant_methods: Some(true),
    })
}

#[cfg(test)]
#[path = "callable_policy_tests.rs"]
mod tests;
