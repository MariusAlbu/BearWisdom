//! Compiler-option spelling is consumed at the independently captured boundary.
use crate::indexer::programs::CompilerIntrinsicPolicy;

pub(crate) fn typescript_options(options: &serde_json::Value) -> Option<CompilerIntrinsicPolicy> {
    let object = options.as_object()?;
    let flag = |key| match object.get(key) {
        None => Some(None),
        Some(value) => value.as_bool().map(Some),
    };
    let strict = flag("strict")?.unwrap_or(false);
    Some(CompilerIntrinsicPolicy {
        strict_iterator_return: flag("strictBuiltinIteratorReturn")?.unwrap_or(strict),
    })
}

#[cfg(test)]
#[path = "compiler_intrinsic_policy_tests.rs"]
mod tests;
