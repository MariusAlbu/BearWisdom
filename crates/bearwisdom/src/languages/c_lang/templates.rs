// =============================================================================
// c_lang/templates.rs  —  Symbol pushers for C++ template declarations,
// concepts, and `using` aliases / namespace imports
// =============================================================================

use super::declarations::{push_function_def, push_specifier};
use super::helpers::{enclosing_scope, extract_doc_comment, node_text};
use super::typerefs::emit_typerefs_for_type_descriptor;
use crate::parser::scope_tree;
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// template_declaration — C++ `template<typename T> class/struct/fn { ... }`
// ---------------------------------------------------------------------------

/// Returns the inner declaration node (class/struct/function/etc) and its
/// optional symbol index after pushing it.  The caller is responsible for
/// recursing into the body.
///
/// We emit one TypeRef per type-parameter constraint when present (e.g.
/// `template<typename T, typename U = int>` → TypeRef to `int`).
pub(super) fn push_template_decl<'a>(
    node: &'a Node<'a>,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    language: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) -> (Option<usize>, Option<Node<'a>>) {
    // The inner declaration is the last named child that is not the template
    // parameter list.
    let mut inner: Option<Node<'a>> = None;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "template_parameter_list" => {
                // emit TypeRef for default type arguments  e.g. `typename T = Foo`
                emit_template_param_typerefs(&child, src, symbols.len(), refs);
            }
            "class_specifier" | "struct_specifier" | "union_specifier"
            | "function_definition" | "alias_declaration" | "declaration"
            | "concept_definition" => {
                inner = Some(child);
            }
            _ => {}
        }
    }

    let inner_node = match inner {
        Some(n) => n,
        None => return (None, None),
    };

    // Push a symbol for the inner declaration.
    let idx = match inner_node.kind() {
        "class_specifier" => {
            push_specifier(&inner_node, src, scope_tree, SymbolKind::Class, symbols, parent_index)
        }
        "struct_specifier" => {
            push_specifier(&inner_node, src, scope_tree, SymbolKind::Struct, symbols, parent_index)
        }
        "union_specifier" => {
            push_specifier(&inner_node, src, scope_tree, SymbolKind::Struct, symbols, parent_index)
        }
        "function_definition" => {
            push_function_def(&inner_node, src, scope_tree, language, symbols, parent_index)
        }
        "concept_definition" => {
            push_concept_def(&inner_node, src, scope_tree, symbols, parent_index)
        }
        _ => None,
    };

    (idx, Some(inner_node))
}

// ---------------------------------------------------------------------------
// concept_definition — C++20 `template<typename T> concept Foo = expr;`
// ---------------------------------------------------------------------------

pub(super) fn push_concept_def(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    // concept_definition: `concept` `identifier` `=` expression
    let name_node = if let Some(n) = node.child_by_field_name("name") {
        n
    } else {
        let mut cursor = node.walk();
        let found = node.children(&mut cursor).find(|c| c.kind() == "identifier");
        found?
    };

    let name = node_text(name_node, src);
    if name.is_empty() {
        return None;
    }

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::TypeAlias, // concepts are type-constraint aliases
        visibility: None,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("concept {name}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });

    Some(idx)
}

fn emit_template_param_typerefs(
    param_list: &Node,
    src: &[u8],
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = param_list.walk();
    for child in param_list.children(&mut cursor) {
        // `optional_type_parameter_declaration` is the node kind for
        // `typename T = SomeType` — it has a default type after `=`.
        // `type_parameter_declaration` is the plain `typename T` variant (no default).
        if child.kind() == "optional_type_parameter_declaration" {
            // Walk children after `=` and emit TypeRef for any named type.
            let mut after_eq = false;
            let mut ic = child.walk();
            for inner in child.children(&mut ic) {
                if inner.kind() == "=" {
                    after_eq = true;
                } else if after_eq {
                    // Could be `type_identifier`, `template_type`, `qualified_identifier`, etc.
                    emit_typerefs_for_type_descriptor(inner, src, source_idx, refs);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// alias_declaration — C++ `using Alias = Type;`
// ---------------------------------------------------------------------------

pub(super) fn push_alias_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    // Structure: `using` <name> `=` `type_descriptor` `;`
    // The name can be: type_identifier, template_type (e.g. `using Foo<T> = Bar`)
    // or qualified_identifier. Skip non-identifier second children.
    let name_node = match node.child(1) {
        Some(n) if matches!(
            n.kind(),
            "type_identifier" | "identifier" | "template_type" | "qualified_identifier"
        ) => n,
        _ => return,
    };
    let name = node_text(name_node, src);
    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::TypeAlias,
        visibility: None,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("using {name} = ...")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });

    // TypeRef for the aliased type.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_descriptor" {
            emit_typerefs_for_type_descriptor(child, src, idx, refs);
        }
    }
}

// ---------------------------------------------------------------------------
// using_declaration — C++ `using std::vector;`  (namespace using, no `=`)
// ---------------------------------------------------------------------------

pub(super) fn push_using_decl(
    node: &Node,
    src: &[u8],
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // The identifier after `using` is a qualified_identifier or identifier.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "qualified_identifier" | "identifier" => {
                let name = node_text(child, src);
                if !name.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index: current_symbol_count,
                        target_name: name,
                        kind: EdgeKind::Imports,
                        line: child.start_position().row as u32,
                        module: None,
                        chain: None,
                        byte_offset: 0,
                                            namespace_segments: Vec::new(),
                                            call_args: Vec::new(),
});
                }
                return;
            }
            _ => {}
        }
    }
}
