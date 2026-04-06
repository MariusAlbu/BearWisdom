// =============================================================================
// sql/primitives.rs — SQL keywords and built-in types
// =============================================================================

/// SQL keywords, data types, and built-in functions that appear as type_ref
/// noise in migration files. These can never resolve to project symbols.
pub(crate) const PRIMITIVES: &[&str] = &[
    // Trigger / special references
    "NEW", "OLD", "NULL", "TRUE", "FALSE", "DEFAULT",
    // Data types
    "INTEGER", "TEXT", "REAL", "BLOB", "BOOLEAN", "TIMESTAMP",
    "VARCHAR", "CHAR", "BIGINT", "SMALLINT", "DECIMAL", "NUMERIC",
    "SERIAL", "UUID", "JSONB", "JSON",
    // Built-in functions
    "count", "sum", "avg", "min", "max", "coalesce", "nullif",
    "now", "current_timestamp", "nextval", "currval",
    "lower", "upper", "trim", "length", "substring",
    "EXTRACT", "CAST", "CASE", "WHEN", "THEN", "ELSE", "END",
    // DDL / DML keywords
    "ALTER", "DROP", "CREATE", "INSERT", "UPDATE", "DELETE", "SELECT",
    "LEFT", "RIGHT", "INNER", "OUTER", "JOIN", "ON", "WHERE", "FROM",
    "GROUP", "ORDER", "BY", "HAVING", "LIMIT", "OFFSET",
    "IF", "EXISTS", "NOT", "AND", "OR", "IN", "LIKE", "BETWEEN",
    "SET", "VALUES", "INTO", "TABLE", "INDEX", "VIEW", "TRIGGER",
    "FUNCTION", "PROCEDURE", "RETURN", "RETURNS", "BEGIN", "DECLARE",
];
