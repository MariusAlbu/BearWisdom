// =============================================================================
// odin/predicates.rs — Odin builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(sym_kind, "method" | "function" | "constructor" | "test"),
        EdgeKind::Inherits => false, // Odin has no inheritance
        EdgeKind::Implements => false,
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "interface" | "enum" | "type_alias" | "function" | "variable"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class" | "function"),
        _ => true,
    }
}

/// Odin's built-in library collections — names a `collection:path` module
/// specifier whose head is a compiler-provided collection, never a project
/// package (project imports are relative paths with no collection prefix).
const ODIN_BUILTIN_COLLECTIONS: &[&str] = &["core", "vendor", "base", "system"];

/// True when a ref's `module` specifier is collection-qualified against a
/// built-in collection (`core:fmt`, `vendor:raylib`, `system:lib`). Drives the
/// profile's `module_skip` so a call/typeref into a foreign collection declines
/// before the strategy ladder rather than binding a same-named project symbol;
/// external classification brands it after.
pub(super) fn is_builtin_collection_module(module: &str) -> bool {
    module
        .split_once(':')
        .is_some_and(|(collection, _)| ODIN_BUILTIN_COLLECTIONS.contains(&collection))
}

#[cfg(test)]
#[path = "predicates_tests.rs"]
mod tests;
