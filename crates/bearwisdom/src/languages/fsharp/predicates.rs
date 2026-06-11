// =============================================================================
// fsharp/predicates.rs — F# builtin and helper predicates
// =============================================================================

/// Fallback external namespace check when no NuGet manifest is available.
/// The closed set of .NET / F# platform namespace roots: the BCL (`System`),
/// the Microsoft platform namespace, and `FSharp` (the FSharp.Core root that
/// ships with every F# toolchain). Third-party package namespaces resolve via
/// the NuGet manifest in `is_manifest_external_namespace`, not here.
pub(super) fn is_external_namespace_fallback(ns: &str) -> bool {
    let root = ns.split('.').next().unwrap_or(ns);
    matches!(root, "System" | "Microsoft" | "FSharp")
}
