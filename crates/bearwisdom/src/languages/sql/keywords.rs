// =============================================================================
// sql/keywords.rs — SQL keywords and built-in types
// =============================================================================

/// SQL keywords, data types, and built-in functions that appear as type_ref
/// noise in migration files. These can never resolve to project symbols.
pub(crate) const KEYWORDS: &[&str] = &[
    // Trigger / special references
    "NEW",
    "OLD",
    "NULL",
    "TRUE",
    "FALSE",
    "DEFAULT",
    // Data types
    "INTEGER",
    "TEXT",
    "REAL",
    "BLOB",
    "BOOLEAN",
    "TIMESTAMP",
    "VARCHAR",
    "CHAR",
    "BIGINT",
    "SMALLINT",
    "DECIMAL",
    "NUMERIC",
    "SERIAL",
    "UUID",
    "JSONB",
    "JSON",
    // Built-in functions
    "count",
    "sum",
    "avg",
    "min",
    "max",
    "coalesce",
    "nullif",
    "now",
    "current_timestamp",
    "nextval",
    "currval",
    "lower",
    "upper",
    "trim",
    "length",
    "substring",
    "EXTRACT",
    "CAST",
    "CASE",
    "WHEN",
    "THEN",
    "ELSE",
    "END",
    // DDL / DML keywords
    "ALTER",
    "DROP",
    "CREATE",
    "INSERT",
    "UPDATE",
    "DELETE",
    "SELECT",
    "LEFT",
    "RIGHT",
    "INNER",
    "OUTER",
    "JOIN",
    "ON",
    "WHERE",
    "FROM",
    "GROUP",
    "ORDER",
    "BY",
    "HAVING",
    "LIMIT",
    "OFFSET",
    "IF",
    "EXISTS",
    "NOT",
    "AND",
    "OR",
    "IN",
    "LIKE",
    "BETWEEN",
    "SET",
    "VALUES",
    "INTO",
    "TABLE",
    "INDEX",
    "VIEW",
    "TRIGGER",
    "FUNCTION",
    "PROCEDURE",
    "RETURN",
    "RETURNS",
    "BEGIN",
    "DECLARE",
];

/// Returns true when `name` is a SQL built-in column type keyword.
pub(crate) fn is_sql_builtin_type(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "int" | "integer" | "bigint" | "smallint" | "tinyint"
            | "numeric" | "decimal" | "float" | "real" | "double"
            | "varchar" | "nvarchar" | "char" | "nchar"
            | "text" | "ntext" | "clob" | "character"
            | "bit" | "boolean" | "bool"
            | "date" | "time" | "timestamp" | "datetime" | "datetime2"
            | "datetimeoffset" | "smalldatetime" | "year" | "interval"
            | "binary" | "varbinary" | "image" | "bytea" | "blob"
            | "json" | "jsonb" | "xml" | "uuid" | "uniqueidentifier"
            | "money" | "smallmoney" | "tinyint" | "mediumint"
            | "unsigned" | "signed" | "zerofill"
            | "serial" | "bigserial" | "smallserial"
            | "array" | "record" | "void" | "unknown"
            | "user-defined" | "rowversion" | "hierarchyid"
            | "geography" | "geometry" | "cursor" | "sql_variant"
            | "table" | "set" | "enum"
    )
}
