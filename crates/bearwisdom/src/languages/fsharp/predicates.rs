// =============================================================================
// fsharp/predicates.rs — F# builtin and helper predicates
// =============================================================================

use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::project_context::ProjectContext;

/// Fallback external namespace check when no NuGet manifest is available.
/// The closed set of .NET / F# platform namespace roots: the BCL (`System`),
/// the Microsoft platform namespace, and `FSharp` (the FSharp.Core root that
/// ships with every F# toolchain). Third-party package namespaces resolve via
/// the NuGet manifest in `is_manifest_external_namespace`, not here.
pub(super) fn is_external_namespace_fallback(ns: &str) -> bool {
    let root = ns.split('.').next().unwrap_or(ns);
    matches!(root, "System" | "Microsoft" | "FSharp")
}

/// The F# `Printf`-format and exception-raising operators: always available
/// without an `open`, defined by the compiler itself in
/// `Microsoft.FSharp.Core.Operators` / `ExtraTopLevelOperators`, and — unlike
/// `box`, `ignore`, `defaultArg`, or the `Option`/`Result` case constructors
/// — never observed shadowed by a project declaration of the same bare name.
/// A target in this set declines the strategy ladder as a language builtin
/// rather than binding to a same-named project symbol.
pub(super) fn is_fsharp_prelude_operator(target: &str) -> bool {
    matches!(
        target,
        "sprintf"
            | "printf"
            | "printfn"
            | "eprintf"
            | "eprintfn"
            | "failwith"
            | "invalidArg"
            | "invalidOp"
            | "reraise"
    )
}

/// Manifest-aware external namespace check. Matches against declared NuGet
/// packages when a manifest is present; falls back to the closed BCL/FSharp
/// set otherwise.
pub(crate) fn is_manifest_external_namespace(ctx: &ProjectContext, ns: &str) -> bool {
    let root = ns.split('.').next().unwrap_or(ns);
    if matches!(root, "System" | "Microsoft") {
        return true;
    }
    if let Some(m) = ctx.manifest(ManifestKind::NuGet) {
        if !m.dependencies.is_empty() {
            if m.dependencies.contains(ns) {
                return true;
            }
            for dep in &m.dependencies {
                if ns.starts_with(dep.as_str())
                    && ns.len() > dep.len()
                    && ns.as_bytes()[dep.len()] == b'.'
                {
                    return true;
                }
                if let Some(dep_root) = dep.split('.').next() {
                    if root == dep_root {
                        return true;
                    }
                }
            }
            return false;
        }
    }
    is_external_namespace_fallback(ns)
}
