use super::predicates;

#[test]
fn laravel_eloquent_methods_not_classified_as_php_builtin() {
    // Laravel Collection fluent API + Eloquent / Query Builder methods are
    // gem-provided third-party APIs indexed by `ecosystem/composer.rs` when
    // the project's `composer.json` declares them. The bare names collide
    // with extremely common method names across non-Laravel PHP code.
    for name in &[
        // Laravel Collection
        "map", "filter", "where", "first", "last", "each", "pluck",
        "collect", "toArray", "toJson", "isEmpty", "isNotEmpty",
        "sortBy", "groupBy", "reject", "contains", "tap", "pipe",
        // Eloquent / Query Builder
        "findOrFail", "find", "create", "update", "delete", "save",
        "refresh", "orderBy", "limit", "offset", "paginate",
        "exists", "doesntExist", "with", "has", "whereHas",
        "belongsTo", "hasMany", "hasOne", "belongsToMany",
    ] {
        assert!(
            !predicates::is_php_builtin(name),
            "{name:?} should not be classified as a php builtin",
        );
    }
}

#[test]
fn real_php_builtins_still_classified() {
    // Sanity: real PHP standard library + exception types still match.
    for name in &[
        // Array / String functions
        "array_map", "array_filter", "count", "in_array", "sort",
        "strlen", "substr", "explode", "implode", "trim",
        // General builtins
        "isset", "empty", "is_null", "is_array", "json_encode",
        "var_dump", "die", "exit", "header",
        // Exceptions
        "Exception", "RuntimeException", "InvalidArgumentException",
        "Throwable", "Error",
    ] {
        assert!(
            predicates::is_php_builtin(name),
            "{name:?} must remain a php builtin",
        );
    }
}
