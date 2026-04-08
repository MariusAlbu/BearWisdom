use std::collections::HashSet;

/// Runtime globals always external for Rust.
/// Crate names commonly seen in attribute paths (#[serde(...)], #[tokio::main]).
pub(crate) const EXTERNALS: &[&str] = &[
    "serde", "async_trait", "tokio", "tracing", "anyhow", "thiserror",
    "clap", "log", "env_logger",
];

/// Dependency-gated framework globals for Rust.
pub(crate) fn framework_globals(deps: &HashSet<String>) -> Vec<&'static str> {
    let mut globals = Vec::new();

    // Diesel ORM DSL
    if deps.contains("diesel") {
        globals.extend(DIESEL_GLOBALS);
    }

    globals
}

const DIESEL_GLOBALS: &[&str] = &[
    "insert_into", "update", "delete", "replace_into",
    "select", "filter", "find", "first", "load", "get_result", "get_results",
    "execute", "returning", "on_conflict",
    "eq", "ne", "gt", "lt", "ge", "le",
    "and", "or", "not", "is_null", "is_not_null",
    "order", "order_by", "then_order_by", "desc", "asc",
    "limit", "offset", "group_by", "having",
    "inner_join", "left_join", "left_outer_join",
    "values", "set", "default_values",
    "as_select", "into_boxed", "distinct",
    "table", "joinable", "allow_tables_to_appear_in_same_query",
    "diesel", "with_lemmy_type", "auto_type", "derive_new",
    "nullable", "is_contained_by", "contains",
];
