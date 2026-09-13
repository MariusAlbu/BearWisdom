// =============================================================================
// php/type_ref_emit.rs  —  one TypeRef for one PHP type name
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef};

/// Type spellings the PHP grammar owns that name no declaration.
const PHP_NON_DECLARATION_TYPES: &[&str] = &[
    "string", "int", "float", "bool", "array", "object", "null", "void", "never", "mixed",
    "callable", "iterable", "self", "static", "parent",
];

/// Emit one TypeRef for a PHP type name spelled as its source writes it.
/// A qualified spelling is preserved: `resolve_type_name_in_scope` and the
/// `ambient_namespace_path` rung both consume the source separator, and the
/// simple leaf alone is ambiguous across namespaces.
pub(super) fn emit_php_type_ref(
    name: &str,
    line: u32,
    byte_offset: u32,
    refs: &mut Vec<ExtractedRef>,
    source_symbol_index: usize,
) {
    let leaf = name.rsplit('\\').next().unwrap_or(name);
    if leaf.is_empty() || PHP_NON_DECLARATION_TYPES.contains(&leaf) {
        return;
    }
    refs.push(ExtractedRef {
        source_symbol_index,
        target_name: name.to_string(),
        kind: EdgeKind::TypeRef,
        line,
        col: 0,
        module: None,
        namespace_segments: Vec::new(),
        chain: None,
        byte_offset,
        call_args: Vec::new(),
        is_import_binding: false,
        is_reexport: false,
        is_include: false,
    });
}
