// SQL language hooks — external classification and file-context construction.

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::legacy::{self as engine, FileContext, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::ParsedFile;

pub struct SqlHooks;

/// SQL/database-engine built-in type and pseudo-function names.
pub(crate) fn is_sql_builtin_type(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "int"
            | "integer"
            | "bigint"
            | "smallint"
            | "tinyint"
            | "numeric"
            | "decimal"
            | "float"
            | "real"
            | "double"
            | "varchar"
            | "nvarchar"
            | "char"
            | "nchar"
            | "text"
            | "ntext"
            | "clob"
            | "character"
            | "tinytext"
            | "mediumtext"
            | "longtext"
            | "citext"
            | "varchar2"
            | "nvarchar2"
            | "blob"
            | "binary"
            | "varbinary"
            | "bytea"
            | "bytes"
            | "image"
            | "tinyblob"
            | "mediumblob"
            | "longblob"
            | "raw"
            | "date"
            | "time"
            | "datetime"
            | "timestamp"
            | "interval"
            | "year"
            | "datetime2"
            | "datetimeoffset"
            | "smalldatetime"
            | "timestamptz"
            | "timetz"
            | "boolean"
            | "bool"
            | "bit"
            | "uuid"
            | "uniqueidentifier"
            | "json"
            | "jsonb"
            | "xml"
            | "money"
            | "smallmoney"
            | "serial"
            | "bigserial"
            | "smallserial"
            | "hierarchyid"
            | "geography"
            | "geometry"
            | "sql_variant"
            | "rowversion"
            | "inet"
            | "cidr"
            | "macaddr"
            | "macaddr8"
            | "tsvector"
            | "tsquery"
            | "void"
            | "null"
            | "unknown"
            | "count"
            | "sum"
            | "avg"
            | "min"
            | "max"
            | "coalesce"
            | "nullif"
            | "cast"
            | "convert"
            | "isnull"
            | "ifnull"
            | "nvl"
    )
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
}

pub static SQL_HOOKS: SqlHooks = SqlHooks;
