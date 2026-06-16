// Prisma language hooks. Absorbed from the deleted `prisma/resolve.rs`.

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::legacy::{self as engine, FileContext, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::ParsedFile;

pub struct PrismaHooks;

/// Prisma built-in scalar types and helper functions.
pub(crate) fn is_prisma_scalar(name: &str) -> bool {
    matches!(
        name,
        "String"
            | "Boolean"
            | "Int"
            | "BigInt"
            | "Float"
            | "Decimal"
            | "DateTime"
            | "Json"
            | "Bytes"
            | "Unsupported"
            | "autoincrement"
            | "cuid"
            | "uuid"
            | "now"
            | "dbgenerated"
            | "auto"
    )
}

impl LanguageEngineHooks for PrismaHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        engine::infer_external_common(file_ctx, ref_ctx, project_ctx, is_prisma_scalar)
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        Some(FileContext {
            file_path: file.path.clone(),
            language: "prisma".to_string(),
            imports: Vec::new(),
            file_namespace: None,
        })
    }
}

pub static PRISMA_HOOKS: PrismaHooks = PrismaHooks;
