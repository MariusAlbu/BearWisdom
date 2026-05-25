// SQL language hooks. Absorbed from the deleted `sql/resolve.rs`.

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, RefContext, Resolution, SymbolLookup,
};
use crate::type_checker::core::DefaultResolver;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct SqlHooks;

/// SQL/database-engine built-in type and pseudo-function names.
pub(crate) fn is_sql_builtin_type(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "int" | "integer" | "bigint" | "smallint" | "tinyint" | "numeric" | "decimal"
            | "float" | "real" | "double"
            | "varchar" | "nvarchar" | "char" | "nchar" | "text" | "ntext" | "clob"
            | "character" | "tinytext" | "mediumtext" | "longtext" | "citext"
            | "varchar2" | "nvarchar2"
            | "blob" | "binary" | "varbinary" | "bytea" | "bytes"
            | "image" | "tinyblob" | "mediumblob" | "longblob" | "raw"
            | "date" | "time" | "datetime" | "timestamp" | "interval" | "year"
            | "datetime2" | "datetimeoffset" | "smalldatetime"
            | "timestamptz" | "timetz"
            | "boolean" | "bool" | "bit"
            | "uuid" | "uniqueidentifier" | "json" | "jsonb" | "xml"
            | "money" | "smallmoney"
            | "serial" | "bigserial" | "smallserial"
            | "hierarchyid" | "geography" | "geometry" | "sql_variant" | "rowversion"
            | "inet" | "cidr" | "macaddr" | "macaddr8"
            | "tsvector" | "tsquery"
            | "void" | "null" | "unknown"
            | "count" | "sum" | "avg" | "min" | "max" | "coalesce" | "nullif"
            | "cast" | "convert" | "isnull" | "ifnull" | "nvl"
    )
}

fn sql_kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::TypeRef => matches!(sym_kind, "struct" | "class" | "function" | "variable"),
        EdgeKind::Calls => matches!(sym_kind, "function" | "method"),
        _ => true,
    }
}

impl LanguageEngineHooks for SqlHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        engine::infer_external_common(file_ctx, ref_ctx, project_ctx, is_sql_builtin_type)
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        Some(FileContext {
            file_path: file.path.clone(),
            language: "sql".to_string(),
            imports: Vec::new(),
            file_namespace: None,
        })
    }

    fn resolve_ref(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        let target = &ref_ctx.extracted_ref.target_name;
        let edge_kind = ref_ctx.extracted_ref.kind;
        if edge_kind != EdgeKind::TypeRef {
            return None;
        }
        if is_sql_builtin_type(target) {
            return None;
        }
        for sym in lookup.by_name(target) {
            if matches!(sym.kind.as_str(), "struct" | "class" | "function") {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "sql_name_lookup",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }
        (DefaultResolver {
            file_ctx,
            ref_ctx,
            lookup,
            kind_compatible: sql_kind_compatible,
        })
        .resolve_all()
    }
}

pub static SQL_HOOKS: SqlHooks = SqlHooks;
