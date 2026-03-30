// =============================================================================
// php/builtins.rs — PHP builtin and helper predicates
// =============================================================================

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
pub(super) fn normalize_php_ns(ns: &str) -> String {
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

    // Check ProjectContext (from composer.json).
    if let Some(ctx) = project_ctx {
        // composer.json package names like "laravel/framework" are stored
        // in external_prefixes. PHP package names often map to namespace roots
        // (e.g., "laravel/framework" → "Illuminate").
        return ctx.is_external_namespace(ns);
    }

    false
}

/// PHP built-in functions, Laravel Collection methods, and Eloquent model methods.
///
/// Covers the PHP standard library functions (always available without `use`),
/// plus the Laravel Collection fluent API and Eloquent ORM methods that appear
/// heavily in PHP project code but are never in the project's own symbol index.
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
            // Laravel Collection fluent API methods
            | "map"
            | "filter"
            | "where"
            | "first"
            | "last"
            | "each"
            | "pluck"
            | "collect"
            | "toArray"
            | "toJson"
            | "isEmpty"
            | "isNotEmpty"
            | "push"
            | "sortBy"
            | "groupBy"
            | "flatten"
            | "unique"
            | "values"
            | "keys"
            | "merge"
            | "reduce"
            | "reject"
            | "contains"
            | "sum"
            | "avg"
            | "min"
            | "max"
            | "chunk"
            | "take"
            | "skip"
            | "tap"
            | "pipe"
            // Eloquent ORM / Query Builder methods
            | "findOrFail"
            | "find"
            | "create"
            | "update"
            | "delete"
            | "save"
            | "refresh"
            | "orderBy"
            | "limit"
            | "offset"
            | "paginate"
            | "get"
            | "all"
            | "exists"
            | "doesntExist"
            | "with"
            | "has"
            | "whereHas"
            | "belongsTo"
            | "hasMany"
            | "hasOne"
            | "belongsToMany"
    )
}
