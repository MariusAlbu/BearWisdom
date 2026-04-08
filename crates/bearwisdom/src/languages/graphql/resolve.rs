// =============================================================================
// languages/graphql/resolve.rs — GraphQL resolution rules
//
// GraphQL has a schema-level type system. All type references are resolved
// against the set of defined types in the same project (potentially across
// multiple `.graphql`/`.gql` files that form a single schema).
//
// References:
//   named_type in field definitions, argument types, return types
//     → TypeRef, target_name = the type name (e.g., "User", "String")
//   implements_interfaces
//     → TypeRef, target_name = the interface name
//
// Resolution strategy:
//   1. Same-file: type defined in the same schema file.
//   2. Global name lookup: type defined across the schema (multi-file schemas).
//   3. Built-in scalars (String, Int, Float, Boolean, ID) are external.
// =============================================================================

use crate::indexer::resolve::engine::{
    FileContext, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

pub struct GraphQlResolver;

impl LanguageResolver for GraphQlResolver {
    fn language_ids(&self) -> &[&str] {
        &["graphql"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        // GraphQL has no import system — all types in the schema are globally
        // visible within the project.
        FileContext {
            file_path: file.path.clone(),
            language: "graphql".to_string(),
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

        // Only process TypeRef edges (named_type, implements_interfaces).
        if edge_kind != EdgeKind::TypeRef {
            return None;
        }

        // GraphQL built-in scalars are never in the project index.
        if is_graphql_builtin(target) {
            return None;
        }

        // Step 1: Same-file resolution.
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name == *target {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "graphql_same_file",
                });
            }
        }

        // Step 2: Global lookup (schema type defined in another file).
        for sym in lookup.by_name(target) {
            if matches!(
                sym.kind.as_str(),
                "class" | "interface" | "enum" | "struct" | "type_alias"
            ) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "graphql_global_type",
                });
            }
        }

        // Fallback: any matching symbol.
        if let Some(sym) = lookup.by_name(target).into_iter().next() {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 0.85,
                strategy: "graphql_global_fallback",
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

        if is_graphql_builtin(target) {
            return Some("graphql".to_string());
        }

        // Introspection types (double-underscore prefix) are built-in.
        if target.starts_with("__") {
            return Some("graphql".to_string());
        }

        None
    }
}

/// GraphQL built-in scalar types and introspection system types.
fn is_graphql_builtin(name: &str) -> bool {
    matches!(
        name,
        "String" | "Int" | "Float" | "Boolean" | "ID"
            | "__Schema" | "__Type" | "__Field" | "__InputValue"
            | "__EnumValue" | "__Directive" | "__DirectiveLocation"
            | "__TypeKind"
    )
}
