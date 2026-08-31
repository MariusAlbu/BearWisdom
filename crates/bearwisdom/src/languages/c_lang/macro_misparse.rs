// =============================================================================
// languages/c_lang/macro_misparse.rs  —  recover Class symbols hidden by macros
// =============================================================================

use tree_sitter::Node;

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};

// ---------------------------------------------------------------------------
// Macro-misparsed class salvage
// ---------------------------------------------------------------------------
//
// When tree-sitter-cpp encounters a class header whose visibility attribute
// is a macro (Qt's `Q_*_EXPORT`, MSVC `__declspec` shims, project-defined
// export macros), the parser commits early to function_definition because
// the unexpanded macro looks like a class name and the real class name
// then looks like a function name. The functions below detect that shape
// at the function_definition node and re-emit a Class symbol for the
// actual identifier.

/// If `node` is a function_definition whose `type` field is a class_specifier
/// with a SCREAMING_SNAKE_CASE `name` field, return the real class
/// identifier — the `identifier` sibling that follows the misparsed
/// class_specifier. Otherwise return None.
pub(super) fn detect_macro_class_misparse(node: &Node, src: &[u8]) -> Option<String> {
    let type_node = node.child_by_field_name("type")?;
    if type_node.kind() != "class_specifier" && type_node.kind() != "struct_specifier" {
        return None;
    }
    let inner_name_node = type_node.child_by_field_name("name")?;
    let inner_name =
        std::str::from_utf8(&src[inner_name_node.start_byte()..inner_name_node.end_byte()]).ok()?;
    if !is_screaming_snake_case(inner_name) {
        return None;
    }
    // The real class identifier is a sibling of `type_node` in the
    // function_definition. Scan children for the first plain identifier
    // that follows the class_specifier child.
    let mut cursor = node.walk();
    let mut after_type = false;
    for child in node.children(&mut cursor) {
        if !after_type {
            if child.id() == type_node.id() {
                after_type = true;
            }
            continue;
        }
        if matches!(
            child.kind(),
            "identifier" | "type_identifier" | "qualified_identifier"
        ) {
            let text = std::str::from_utf8(&src[child.start_byte()..child.end_byte()]).ok()?;
            if !text.is_empty() && !is_screaming_snake_case(text) {
                return Some(text.to_string());
            }
        }
        // Stop scanning once we cross into the body or an ERROR (`: public Foo`).
        if matches!(child.kind(), "compound_statement" | "ERROR") {
            break;
        }
    }
    None
}

fn is_screaming_snake_case(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let mut has_underscore = false;
    for ch in name.chars() {
        if ch == '_' {
            has_underscore = true;
            continue;
        }
        if !(ch.is_ascii_uppercase() || ch.is_ascii_digit()) {
            return false;
        }
    }
    has_underscore
}

/// Emit the salvaged Class symbol with the recovered name. Mirrors
/// `push_specifier`'s shape: scope-qualified name, signature `class X`,
/// scope_path inherited from the enclosing namespace.
pub(super) fn push_misparsed_class(
    node: &Node,
    real_name: &str,
    src: &[u8],
    scope_tree: &crate::parser::scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    use crate::parser::scope_tree as st;
    let scope = super::helpers::enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = st::qualify(real_name, scope);
    let scope_path = st::scope_path(scope);
    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: real_name.to_string(),
        qualified_name,
        kind: SymbolKind::Class,
        visibility: None,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("class {real_name}")),
        doc_comment: super::helpers::extract_doc_comment(node, src),
        scope_path,
        parent_index,
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    });
    Some(idx)
}

/// In the misparse, the inheritance clause `: public QDialog` lands inside
/// an ERROR node that's a sibling of the recovered identifier. Walk that
/// ERROR's children for `identifier`/`type_identifier`/`qualified_identifier`
/// nodes and emit Inherits refs against the salvaged class symbol.
pub(super) fn emit_misparsed_base_class_refs(
    node: &Node,
    src: &[u8],
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "ERROR" {
            continue;
        }
        let mut ec = child.walk();
        for inner in child.children(&mut ec) {
            if matches!(
                inner.kind(),
                "identifier" | "type_identifier" | "qualified_identifier"
            ) {
                let text = match std::str::from_utf8(&src[inner.start_byte()..inner.end_byte()]) {
                    Ok(t) => t,
                    Err(_) => continue,
                };
                if text.is_empty() || matches!(text, "public" | "private" | "protected" | "virtual")
                {
                    continue;
                }
                refs.push(ExtractedRef {
                    is_include: false,
                    is_import_binding: false,
                    is_reexport: false,
                    source_symbol_index: source_idx,
                    target_name: text.to_string(),
                    kind: EdgeKind::Inherits,
                    line: inner.start_position().row as u32,
                    col: 0,
                    module: None,
                    chain: None,
                    byte_offset: inner.start_byte() as u32,
                    namespace_segments: Vec::new(),
                    call_args: Vec::new(),
                });
            }
        }
    }
}
