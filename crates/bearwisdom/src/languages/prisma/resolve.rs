// =============================================================================
// languages/prisma/resolve.rs — Prisma schema resolution rules
//
// Prisma PSL references:
//
//   model User { posts Post[] }     → TypeRef, target_name = "Post"
//   model Post { author User }      → TypeRef, target_name = "User"
//   model Order { status OrderStatus } → TypeRef, target_name = "OrderStatus"
//   @relation(references: [id])     → TypeRef edges via the field type
//
// Resolution strategy:
//   1. Same-file: Prisma schemas are typically a single file; all types
//      (model, enum, type) defined in the file are in scope.
//   2. Global name lookup: for split schema files (prisma multi-file feature).
//   3. Prisma built-in scalar types are external.
// =============================================================================

use crate::indexer::resolve::engine::{
    self as engine, FileContext, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

pub struct PrismaResolver;

impl LanguageResolver for PrismaResolver {
    fn language_ids(&self) -> &[&str] {
        &["prisma"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        // Prisma has no import statements — all types in a schema are globally
        // visible. Multi-file Prisma schemas are treated as a flat namespace.
        FileContext {
            file_path: file.path.clone(),
            language: "prisma".to_string(),
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

        // Only process TypeRef edges (model/enum references in field types).
        if edge_kind != EdgeKind::TypeRef {
            return None;
        }

        // Prisma built-in scalar types are never in the project index.
        if is_prisma_scalar(target) {
            return None;
        }

        engine::resolve_common(
            "prisma",
            file_ctx,
            ref_ctx,
            lookup,
            |_edge_kind, sym_kind| {
                matches!(sym_kind, "struct" | "enum" | "class" | "type_alias")
            },
        )
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        // Language-specific: provider attribute paths like @db.Text are Prisma-external.
        if target.starts_with("@db.") || target.starts_with("db.") {
            return Some("prisma".to_string());
        }

        engine::infer_external_common(file_ctx, ref_ctx, project_ctx, is_prisma_scalar)
    }
}

/// Prisma built-in scalar types.
fn is_prisma_scalar(name: &str) -> bool {
    matches!(
        name,
        "String" | "Boolean" | "Int" | "BigInt" | "Float"
            | "Decimal" | "DateTime" | "Json" | "Bytes"
            | "Unsupported"
            // Prisma utility types
            | "autoincrement" | "cuid" | "uuid" | "now" | "dbgenerated"
            | "auto"
    )
}
