// Prisma language hooks. Absorbed from the deleted `prisma/resolve.rs`.

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, RefContext, Resolution, SymbolLookup,
};
use crate::type_checker::core::DefaultResolver;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

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

    fn resolve_ref(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        if ref_ctx.extracted_ref.kind != EdgeKind::TypeRef {
            return None;
        }
        if is_prisma_scalar(&ref_ctx.extracted_ref.target_name) {
            return None;
        }
        (DefaultResolver {
            file_ctx,
            ref_ctx,
            lookup,
            kind_compatible: |_, sym_kind| {
                matches!(sym_kind, "struct" | "enum" | "class" | "type_alias")
            },
        })
        .resolve_all()
    }
}

pub static PRISMA_HOOKS: PrismaHooks = PrismaHooks;
