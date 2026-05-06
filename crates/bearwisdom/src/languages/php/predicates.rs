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

/// Normalize PHP namespace separator `\` to `.` for index consistency.
/// "App\\Models\\User" → "App.Models.User"
pub(crate) fn normalize_php_ns(ns: &str) -> String {
    // Trim leading backslash (global namespace qualifier: `\App\Models\User`).
    let trimmed = ns.trim_start_matches('\\');
    trimmed.replace('\\', ".")
}

/// Always-external PHP namespace roots (frameworks + major libraries).
const ALWAYS_EXTERNAL: &[&str] = &[
    "Illuminate",   // Laravel
    "Symfony",      // Symfony
    "Doctrine",     // Doctrine ORM
    "PHPUnit",      // PHPUnit
    "Psr",          // PSR interfaces
    "GuzzleHttp",   // Guzzle HTTP
    "Carbon",       // Carbon date
    "Monolog",      // Monolog logging
];

/// Check whether a PHP namespace (dotted form) is external.
pub(super) fn is_external_php_namespace(
    ns: &str,
    project_ctx: Option<&ProjectContext>,
) -> bool {
    // Always-external first.
    for prefix in ALWAYS_EXTERNAL {
        if ns == *prefix || ns.starts_with(&format!("{prefix}.")) {
            return true;
        }
    }

    // Check Composer manifest directly.
    if let Some(ctx) = project_ctx {
        return is_manifest_php_external(ctx, ns);
    }

    false
}

/// Check whether a PHP namespace is external using the Composer manifest directly.
pub(super) fn is_manifest_php_external(ctx: &ProjectContext, ns: &str) -> bool {
    let root = ns.split('.').next().unwrap_or(ns);
    // Always-external check.
    for prefix in ALWAYS_EXTERNAL {
        if ns == *prefix || ns.starts_with(&format!("{prefix}.")) {
            return true;
        }
    }
    if let Some(m) = ctx.manifest(ManifestKind::Composer) {
        if m.dependencies.contains(ns) {
            return true;
        }
        for dep in &m.dependencies {
            // Composer package names use "vendor/package" form; namespace roots are
            // the second segment (e.g., "laravel/framework" → namespace root "Illuminate").
            // We match against the namespace root segment.
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

