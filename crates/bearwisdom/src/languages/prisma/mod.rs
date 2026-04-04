//! Prisma schema language plugin.
//!
//! Grammar: tree-sitter-prisma v0.1.1 requires tree-sitter >= 0.19, < 0.21 (old ABI —
//! incompatible with tree-sitter 0.25). `grammar()` returns `None` until a
//! compatible release is published. Extraction is performed by a line-oriented
//! regex-free parser that handles the Prisma PSL format directly.
//!
//! What we extract:
//! - `model`       → Struct (ORM table)
//! - `view`        → Class (database view)
//! - `enum`        → Enum
//! - `type`        → TypeAlias (composite type)
//! - `datasource`  → Variable (DB connection config)
//! - `generator`   → Variable (client generator config)
//! - Enum values   → EnumMember (children of Enum)
//! - Model fields  → Field (children of Struct/Class)
//! - Field types   → TypeRef edges to Prisma model/enum types
//! - `@relation`   → TypeRef to referenced model

pub mod extract;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct PrismaPlugin;

impl LanguagePlugin for PrismaPlugin {
    fn id(&self) -> &str {
        "prisma"
    }

    fn language_ids(&self) -> &[&str] {
        &["prisma"]
    }

    fn extensions(&self) -> &[&str] {
        &[".prisma"]
    }

    /// TODO: wire in tree_sitter_prisma::language() once the crate is updated to
    /// tree-sitter 0.22+ (currently requires >= 0.19, < 0.21 — ABI-incompatible).
    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        None
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
    }

    fn extract(&self, source: &str, file_path: &str, _lang_id: &str) -> ExtractionResult {
        let _ = file_path;
        extract::extract(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        // No grammar, but list the logical constructs for coverage tooling.
        &["model_declaration", "enum_declaration", "datasource_declaration", "generator_declaration", "type_declaration"]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &["column_declaration", "enumeral"]
    }

    fn builtin_type_names(&self) -> &[&str] {
        // Prisma scalar types — no TypeRef should be emitted for these.
        &[
            "String", "Int", "Float", "Boolean", "DateTime",
            "Bytes", "Json", "BigInt", "Decimal",
        ]
    }
}
