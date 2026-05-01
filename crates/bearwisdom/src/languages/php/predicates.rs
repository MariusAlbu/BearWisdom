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

/// PHP built-in functions and exception types always available without
/// `use`. Laravel Collection / Eloquent ORM / Query Builder methods are
/// gem-provided and indexed by `ecosystem/composer.rs` when the project's
/// `composer.json` declares the corresponding packages — they don't belong
/// in this predicate.
pub(super) fn is_php_builtin(name: &str) -> bool {
    let root = name.split(['.', ':']).next().unwrap_or(name);
    matches!(
        root,
        // Array functions
        "array_map"
            | "array_filter"
            | "array_reduce"
            | "array_merge"
            | "array_push"
            | "array_pop"
            | "array_shift"
            | "array_unshift"
            | "array_keys"
            | "array_values"
            | "array_unique"
            | "array_reverse"
            | "array_slice"
            | "array_splice"
            | "array_search"
            | "array_flip"
            | "array_walk"
            | "array_chunk"
            | "array_combine"
            | "array_diff"
            | "array_intersect"
            | "count"
            | "sizeof"
            | "in_array"
            | "sort"
            | "asort"
            | "ksort"
            | "usort"
            // String functions
            | "strlen"
            | "strpos"
            | "strrpos"
            | "substr"
            | "str_replace"
            | "str_contains"
            | "str_starts_with"
            | "str_ends_with"
            | "strtolower"
            | "strtoupper"
            | "trim"
            | "ltrim"
            | "rtrim"
            | "explode"
            | "implode"
            | "sprintf"
            | "printf"
            | "print"
            | "number_format"
            | "ucfirst"
            | "lcfirst"
            // General functions
            | "isset"
            | "empty"
            | "is_null"
            | "is_array"
            | "is_string"
            | "is_numeric"
            | "is_int"
            | "is_float"
            | "is_bool"
            | "is_object"
            | "json_encode"
            | "json_decode"
            | "var_dump"
            | "print_r"
            | "var_export"
            | "die"
            | "exit"
            | "header"
            | "setcookie"
            | "session_start"
            | "intval"
            | "floatval"
            | "strval"
            | "boolval"
            | "date"
            | "time"
            | "mktime"
            | "strtotime"
            | "file_get_contents"
            | "file_put_contents"
            | "file_exists"
            | "ob_start"
            | "ob_get_clean"
            | "class_exists"
            | "interface_exists"
            | "method_exists"
            | "property_exists"
            | "get_class"
            | "get_parent_class"
            | "is_a"
            | "instanceof"
            // Exception types (always available without import)
            | "Exception"
            | "RuntimeException"
            | "InvalidArgumentException"
            | "BadMethodCallException"
            | "LogicException"
            | "Throwable"
            | "Error"
    )
}
