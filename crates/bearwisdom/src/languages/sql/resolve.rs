// =============================================================================
// languages/sql/resolve.rs — SQL resolution rules
//
// SQL references (TypeRef via object_reference) are bare table/view names:
//
//   FOREIGN KEY REFERENCES orders(id)  → target_name = "orders"
//   ALTER TABLE orders ...             → target_name = "orders"
//   CREATE VIEW v AS SELECT * FROM t   → target_name = "t"
//
// Resolution strategy:
//   1. Look up the name directly via `lookup.by_name()` — SQL symbols use
//      bare names as their qualified_name. Prefer tables (Struct) and views
//      (Class) over other symbol kinds. Any match in the same project resolves.
//   2. No namespace / import tracking needed — SQL is single-schema by default.
//   3. `infer_external_namespace` marks stdlib SQL keywords as "builtin" so
//      they leave the unresolved_refs bucket.
// =============================================================================

use crate::indexer::resolve::engine::{
    self as engine, FileContext, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

pub struct SqlResolver;

impl LanguageResolver for SqlResolver {
    fn language_ids(&self) -> &[&str] {
        &["sql"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        // SQL has no imports and no file-level namespace.
        FileContext {
            file_path: file.path.clone(),
            language: "sql".to_string(),
            imports: Vec::new(),
            file_namespace: None,
        }
    }

    fn resolve(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        let target = &ref_ctx.extracted_ref.target_name;
        let edge_kind = ref_ctx.extracted_ref.kind;

        // Only handle TypeRef (object references to tables/views/functions).
        if edge_kind != EdgeKind::TypeRef {
            return None;
        }

        // Skip SQL built-in types — they won't be in the symbol index.
        if is_sql_builtin_type(target) {
            return None;
        }

        // Direct lookup by name — SQL symbols have bare names as qualified_name.
        // Prefer tables (Struct) and views (Class) for TypeRef edges.
        for sym in lookup.by_name(target) {
            if matches!(sym.kind.as_str(), "struct" | "class" | "function") {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "sql_name_lookup",
                });
            }
        }

        // Fall through to common resolution (handles qualified names, same-file).
        engine::resolve_common("sql", file_ctx, ref_ctx, lookup, sql_kind_compatible)
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        engine::infer_external_common(file_ctx, ref_ctx, project_ctx, is_sql_builtin_type)
    }
}

/// Edge-kind / symbol-kind compatibility for SQL.
/// SQL refs are almost exclusively TypeRef (table/view/function references).
fn sql_kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::TypeRef => matches!(sym_kind, "struct" | "class" | "function" | "variable"),
        EdgeKind::Calls => matches!(sym_kind, "function" | "method"),
        _ => true,
    }
}

/// SQL/database-engine built-in type and pseudo-function names.
/// These appear as TypeRef targets (column types) and should not be resolved
/// against the project symbol table.
fn is_sql_builtin_type(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        // Numeric
        "int" | "integer" | "bigint" | "smallint" | "tinyint" | "numeric" | "decimal"
            | "float" | "real" | "double"
            // String
            | "varchar" | "nvarchar" | "char" | "nchar" | "text" | "ntext" | "clob"
            | "character"
            // Binary
            | "blob" | "binary" | "varbinary" | "bytea" | "bytes"
            // Date/time
            | "date" | "time" | "datetime" | "timestamp" | "interval" | "year"
            // Boolean
            | "boolean" | "bool" | "bit"
            // Misc
            | "uuid" | "json" | "jsonb" | "xml" | "money" | "smallmoney"
            | "serial" | "bigserial" | "smallserial"
            | "void" | "null" | "unknown"
            // Aggregate / pseudo-functions sometimes captured as refs
            | "count" | "sum" | "avg" | "min" | "max" | "coalesce" | "nullif"
            | "cast" | "convert" | "isnull" | "ifnull" | "nvl"
    )
}
