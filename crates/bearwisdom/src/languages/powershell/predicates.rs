// =============================================================================
// powershell/predicates.rs — PowerShell builtin-name predicate
// =============================================================================

/// Does `name` match a PowerShell core-module cmdlet, reserved keyword, or
/// primitive type name? PowerShell command and parameter names are
/// case-insensitive at every call site, so the comparison folds case.
pub(super) fn is_powershell_builtin(name: &str) -> bool {
    super::builtins::CORE_BUILTINS
        .iter()
        .any(|builtin| builtin.eq_ignore_ascii_case(name))
}

#[cfg(test)]
#[path = "predicates_tests.rs"]
mod tests;
