// =============================================================================
// php/predicates.rs — PHP builtin and helper predicates
// =============================================================================

use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::project_context::ProjectContext;
use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(
            sym_kind,
            "method" | "function" | "constructor" | "test" | "property"
        ),
        EdgeKind::Inherits => matches!(sym_kind, "class"),
        EdgeKind::Implements => matches!(sym_kind, "interface"),
        // PHP traits use "class" kind in the index.
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "interface" | "enum" | "type_alias" | "namespace"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class"),
        _ => true,
    }
}

/// PHP language constructs that are never project symbols. Declined before the
/// ladder so a same-named project function can't bind them. Delegates to the
/// closed `CONSTRUCTS` table in `php/keywords.rs` — single source of truth.
pub(super) fn is_php_builtin(name: &str) -> bool {
    super::keywords::CONSTRUCTS.contains(&name)
}

/// Normalize PHP namespace separator `\` to `.` for index consistency.
/// "App\\Models\\User" → "App.Models.User"
pub(crate) fn normalize_php_ns(ns: &str) -> String {
    // Trim leading backslash (global namespace qualifier: `\App\Models\User`).
    let trimmed = ns.trim_start_matches('\\');
    trimmed.replace('\\', ".")
}

/// Check whether a PHP namespace (dotted form) is external. A namespace is
/// external only when the project's `composer.json` declares the owning
/// package — there is no hardcoded framework list. With no Composer manifest
/// on disk, the namespace is left unresolved (honestly) rather than guessed.
pub(super) fn is_external_php_namespace(ns: &str, project_ctx: Option<&ProjectContext>) -> bool {
    if let Some(ctx) = project_ctx {
        return is_manifest_php_external(ctx, ns);
    }

    false
}

/// Check whether a PHP namespace is external using the Composer manifest directly.
pub(super) fn is_manifest_php_external(ctx: &ProjectContext, ns: &str) -> bool {
    let root = ns.split('.').next().unwrap_or(ns);
    if let Some(m) = ctx.manifest(ManifestKind::Composer) {
        if m.dependencies.contains(ns) {
            return true;
        }
        for dep in &m.dependencies {
            // Composer package names use "vendor/package" form. Match the
            // namespace root segment against the package segment (the part
            // after "/"), and also against the full "vendor/package" prefix.
            let dep_ns_root = dep.split('/').nth(1).unwrap_or(dep.as_str());
            if root == dep_ns_root {
                return true;
            }
            if ns.starts_with(dep.as_str()) {
                return true;
            }
        }
    }
    false
}
