// =============================================================================
// c_lang/preproc.rs  —  Symbol pushers for preprocessor `#define` directives
// =============================================================================

use super::helpers::{enclosing_scope, extract_doc_comment, node_text};
use crate::parser::scope_tree;
use crate::types::{ExtractedSymbol, SymbolKind};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// preproc_def — `#define FOO value`  → Constant/Variable
// ---------------------------------------------------------------------------

pub(super) fn push_preproc_def(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    // Children: `#define`, `identifier`, optional `preproc_arg`
    let name_node = match node.child(1) {
        Some(n) if n.kind() == "identifier" => n,
        _ => return,
    };
    let name = node_text(name_node, src);
    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let value = node
        .child(2)
        .filter(|n| n.kind() == "preproc_arg")
        .map(|n| node_text(n, src));
    let signature = Some(match &value {
        Some(v) => format!("#define {name} {v}"),
        None => format!("#define {name}"),
    });

    symbols.push(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::Variable,
        visibility: None,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature,
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
            byte_offset: 0,
    declared_type: None,
    return_type: None,
    param_types: Vec::new(),
    generic_params: Vec::new(),
});
}

// ---------------------------------------------------------------------------
// preproc_function_def — `#define MAX(a, b) ...`  → Function
// ---------------------------------------------------------------------------

pub(super) fn push_preproc_function_def(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    // Children: `#define`, `identifier`, `preproc_params`, optional `preproc_arg`
    let name_node = match node.child(1) {
        Some(n) if n.kind() == "identifier" => n,
        _ => return,
    };
    let name = node_text(name_node, src);
    let params = node
        .child(2)
        .filter(|n| n.kind() == "preproc_params")
        .map(|n| node_text(n, src))
        .unwrap_or_default();

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Function,
        visibility: None,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("#define {name}{params}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
            byte_offset: 0,
    declared_type: None,
    return_type: None,
    param_types: Vec::new(),
    generic_params: Vec::new(),
});
}
