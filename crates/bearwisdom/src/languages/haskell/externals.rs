use std::collections::HashSet;

/// Runtime globals always external for Haskell.
///
/// NOTE: With the import walk in `infer_external_common`, most third-party
/// library names (HSpec, Aeson, Megaparsec) are now classified via their
/// import statements. This list only needs Prelude/stdlib names that appear
/// without any import — but those are already handled by `is_haskell_builtin`
/// in builtins.rs.
///
/// What remains: qualified module-prefix names (Map.*, Set.*, Text.*) that
/// the import walk handles via module-qualified matching, and operators that
/// don't map to any import entry.
///
/// This list is intentionally thin — the import walk does the heavy lifting.
pub(crate) const EXTERNALS: &[&str] = &[
    // Operators that may not have explicit imports (Prelude re-exports)
    "<>", ".", "<$>", "<*>", "<|>", "<$", "$>",
    "++", "==", "/=", ">>=", ">>", "=<<",
    "$", "&&", "||",
    ">=", "<=", ">", "<",
    "+", "-", "*", "/", "^", "**", "^^",
    "!!", ":", "@",
    "<=<", ">=>",
];

/// Dependency-gated framework globals for Haskell.
///
/// NOTE: With the import walk active, test framework names (it, describe,
/// shouldBe, etc.) are classified via `import Test.Hspec`. These lists are
/// now a redundant safety net and can be removed once the import walk is
/// validated across more projects.
pub(crate) fn framework_globals(_deps: &HashSet<String>) -> Vec<&'static str> {
    // Import walk handles all framework globals via import tracing.
    // Keeping the function signature for API compatibility.
    Vec::new()
}
