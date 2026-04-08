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
    FileContext, LanguageResolver, RefContext, Resolution, SymbolLookup,
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

        // Step 1: Same-file resolution (most Prisma projects use one schema file).
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name == *target {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "prisma_same_file",
                });
            }
        }

        // Step 2: Global lookup (multi-file Prisma schemas).
        for sym in lookup.by_name(target) {
            if matches!(sym.kind.as_str(), "struct" | "enum" | "class" | "type_alias") {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "prisma_global_type",
                });
            }
        }

        // Step 3: Any matching symbol.
        if let Some(sym) = lookup.by_name(target).into_iter().next() {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 0.85,
                strategy: "prisma_global_fallback",
            });
        }

        None
    }

    fn infer_external_namespace(
        &self,
        _file_ctx: &FileContext,
        ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        if is_prisma_scalar(target) {
            return Some("prisma".to_string());
        }

        // Prisma attribute functions like @db.Text, @db.VarChar are provider-specific.
        if target.starts_with("@db.") || target.starts_with("db.") {
            return Some("prisma".to_string());
        }

        None
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
