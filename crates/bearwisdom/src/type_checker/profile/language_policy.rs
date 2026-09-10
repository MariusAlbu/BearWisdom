//! Bridges generic resolver operations to language-owned policy.
//!
//! The resolver asks only for a semantic operation. Surface spellings and
//! declaration syntax stay with the language that owns them.

use crate::type_checker::core::types::TypeArena;
use crate::type_checker::profile::language_profile::LanguageProfile;
use crate::types::{AliasTargetIds, CallArg};

pub use super::chain_specs::{AliasIntrinsic, CallbackArgumentPolicy};

/// Ask language policy how a named alias participates in member resolution.
pub fn alias_intrinsic(profile: Option<&LanguageProfile>, head: &str) -> Option<AliasIntrinsic> {
    crate::languages::default_registry()
        .get_dedicated(profile?.id)
        .and_then(|plugin| plugin.alias_intrinsic(head))
}

/// Decode a flat type expression that extracts a callable's return value.
pub fn flat_callable_return_operand<'a>(
    profile: Option<&LanguageProfile>,
    head: &'a str,
) -> Option<&'a str> {
    crate::languages::default_registry()
        .get_dedicated(profile?.id)
        .and_then(|plugin| plugin.flat_callable_return_operand(head))
}

/// Decode the callable operand stored in an applied return extractor.
pub fn callable_return_operand<'a>(
    profile: Option<&LanguageProfile>,
    head: &'a str,
) -> Option<&'a str> {
    crate::languages::default_registry()
        .get_dedicated(profile?.id)
        .and_then(|plugin| plugin.callable_return_operand(head))
}

/// Whether an alias target has the language's callable-return extraction shape.
pub fn is_callable_return_extractor(
    profile: Option<&LanguageProfile>,
    arena: &TypeArena,
    target: &AliasTargetIds,
) -> bool {
    crate::languages::default_registry()
        .get_dedicated(match profile {
            Some(profile) => profile.id,
            None => return false,
        })
        .is_some_and(|plugin| plugin.is_callable_return_extractor(arena, target))
}

/// The policy for a declaration's parameter position. Each language gets a
/// chance to recognize its own declaration file and signature surface.
pub fn callback_argument_policy(
    file_path: &str,
    signature: Option<&str>,
    index: usize,
) -> CallbackArgumentPolicy {
    let registry = crate::languages::default_registry();
    registry
        .language_by_extension(file_path)
        .and_then(|language| registry.get_dedicated(language))
        .and_then(|plugin| plugin.callback_argument_policy(file_path, signature, index))
        .unwrap_or(CallbackArgumentPolicy::Any)
}

/// Whether an extracted callback argument satisfies a semantic policy.
pub fn callback_argument_is_compatible(policy: CallbackArgumentPolicy, arg: &CallArg) -> bool {
    match policy {
        CallbackArgumentPolicy::Any => matches!(
            arg,
            CallArg::Lambda { .. } | CallArg::LambdaAt { .. } | CallArg::TrailingBlockAt { .. }
        ),
        CallbackArgumentPolicy::TrailingBlockOnly => matches!(arg, CallArg::TrailingBlockAt { .. }),
        CallbackArgumentPolicy::PositionalOnly => matches!(arg, CallArg::LambdaAt { .. }),
    }
}
