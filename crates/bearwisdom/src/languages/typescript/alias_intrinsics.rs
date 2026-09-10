//! TypeScript compiler intrinsics used by resolver alias expansion.

use crate::type_checker::core::types::TypeArena;
use crate::type_checker::profile::chain_specs::AliasIntrinsic;
use crate::types::AliasTargetIds;

/// Return the member-resolution meaning of a TypeScript utility type.
pub(crate) fn member_resolution_intrinsic(head: &str) -> Option<AliasIntrinsic> {
    match head {
        "NoInfer" | "Omit" | "Pick" | "Partial" | "Required" | "Readonly" | "NonNullable"
        | "Awaited" => Some(AliasIntrinsic::TransparentFirstArgument),
        "ReturnType" => Some(AliasIntrinsic::CallableReturn),
        _ => None,
    }
}

/// Decode the flattened `ReturnType<typeof f>` form retained in a class head.
pub(crate) fn flat_callable_return_operand(head: &str) -> Option<&str> {
    head.strip_prefix("ReturnType<typeof ")
        .and_then(|source| source.strip_suffix('>'))
        .map(str::trim)
}

/// Decode the value form accepted by an applied TypeScript return extractor.
pub(crate) fn callable_return_operand(head: &str) -> Option<&str> {
    head.strip_prefix("typeof ").map(str::trim)
}

/// Whether an alias target has TypeScript's `ReturnType<T>` conditional shape.
pub(crate) fn is_callable_return_extractor(arena: &TypeArena, target: &AliasTargetIds) -> bool {
    let AliasTargetIds::Conditional {
        extends,
        true_branch,
        ..
    } = target
    else {
        return false;
    };
    let extends_str = arena.format_type(*extends);
    let Some((_, infer_tail)) = extends_str.rsplit_once("=> infer ") else {
        return false;
    };
    let infer_var = infer_tail
        .trim_start()
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .next()
        .unwrap_or("");
    !infer_var.is_empty() && infer_var == arena.format_type(*true_branch).trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_member_preserving_and_callable_return_intrinsics() {
        assert_eq!(
            member_resolution_intrinsic("Omit"),
            Some(AliasIntrinsic::TransparentFirstArgument)
        );
        assert_eq!(
            member_resolution_intrinsic("ReturnType"),
            Some(AliasIntrinsic::CallableReturn)
        );
        assert_eq!(member_resolution_intrinsic("ProjectAlias"), None);
    }

    #[test]
    fn decodes_flat_and_applied_callable_operands() {
        assert_eq!(
            flat_callable_return_operand("ReturnType<typeof createClient>"),
            Some("createClient")
        );
        assert_eq!(
            callable_return_operand("typeof createClient"),
            Some("createClient")
        );
        assert_eq!(flat_callable_return_operand("ProjectAlias"), None);
    }
}
