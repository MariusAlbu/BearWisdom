use super::calls::build_chain;
use super::helpers::{detect_visibility, node_text};
use super::types::extract_type_ref_from_annotation;
use crate::parser::scope_tree;
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::Node;

/// Extract constructor parameter properties.
///
/// In TypeScript, `constructor(private db: DatabaseRepository)` is shorthand
/// for declaring a class property `db` of type `DatabaseRepository`.
/// Tree-sitter represents this as:
///
/// ```text
/// method_definition [constructor]
///   formal_parameters
///     required_parameter
///       accessibility_modifier  ← "private"/"public"/"protected"/"readonly"
///       identifier "db"
///       type_annotation
///         type_identifier "DatabaseRepository"
/// ```
///
/// For each such parameter, we emit:
/// 1. A property symbol (`AlbumService.db`)
/// 2. A TypeRef ref from the property to the type name
pub(super) fn extract_constructor_params(
    method_node: &Node,
    src: &[u8],
    scope_tree: &crate::parser::scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let params = match method_node.child_by_field_name("parameters") {
        Some(p) => p,
        None => return,
    };

    let mut cursor = params.walk();
    for param in params.children(&mut cursor) {
        if param.kind() != "required_parameter" {
            continue;
        }

        // Check for accessibility modifier (private/public/protected/readonly).
        let has_modifier = param
            .children(&mut param.walk())
            .any(|c| c.kind() == "accessibility_modifier" || c.kind() == "readonly");
        if !has_modifier {
            continue;
        }

        // Get the parameter name.
        let name_node = match param.child_by_field_name("pattern") {
            Some(n) => n,
            None => continue,
        };
        let name = node_text(name_node, src);

        // Build qualified name relative to the class scope.
        let parent_scope = if method_node.start_byte() > 0 {
            scope_tree::find_scope_at(scope_tree, method_node.start_byte() - 1)
        } else {
            None
        };
        let qualified_name = scope_tree::qualify(&name, parent_scope);
        let scope_path = scope_tree::scope_path(parent_scope);

        let prop_idx = symbols.len();
        symbols.push(ExtractedSymbol {
            name: name.clone(),
            qualified_name,
            kind: SymbolKind::Property,
            visibility: detect_visibility(&param, src),
            start_line: param.start_position().row as u32,
            end_line: param.end_position().row as u32,
            start_col: param.start_position().column as u32,
            end_col: param.end_position().column as u32,
            signature: None,
            doc_comment: None,
            scope_path,
            parent_index,
        });

        // Extract TypeRef from the type annotation.
        if let Some(type_ann) = param.child_by_field_name("type") {
            extract_type_ref_from_annotation(&type_ann, src, prop_idx, refs);
        }
    }
}

/// Extract the catch variable from a `catch_clause` as a Variable symbol.
///
/// Handles `catch (e: Error)` and `catch (e)`:
/// ```text
/// catch_clause
///   "catch"
///   catch_parameter   (or just identifier in some grammars)
///     identifier "e"
///     type_annotation
///       ":"
///       type_identifier "Error"
///   statement_block
/// ```
///
/// Emits a Variable symbol for `e` and, if typed, a TypeRef to the catch type.
pub(super) fn extract_catch_variable(
    node: &Node,
    src: &[u8],
    scope_tree: &crate::parser::scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    // Find the catch parameter node. Tree-sitter may represent it as a
    // `catch_parameter` child, or as a bare `identifier` after `catch`.
    let mut param_node: Option<Node> = None;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "catch_parameter" | "identifier" => {
                param_node = Some(child);
                break;
            }
            _ => {}
        }
    }
    let Some(param) = param_node else { return };

    // Extract the identifier name — either the param itself (if `identifier`)
    // or its first `identifier` child (if `catch_parameter`).
    let name_node = if param.kind() == "identifier" {
        param
    } else {
        // Inside catch_parameter, find the identifier child.
        let mut found: Option<Node> = None;
        let mut pcursor = param.walk();
        for child in param.children(&mut pcursor) {
            if child.kind() == "identifier" {
                found = Some(child);
                break;
            }
        }
        match found {
            Some(n) => n,
            None => return,
        }
    };

    let name = node_text(name_node, src);
    if name.is_empty() {
        return;
    }

    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let qualified_name = scope_tree::qualify(&name, parent_scope);
    let scope_path = scope_tree::scope_path(parent_scope);

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Variable,
        visibility: None,
        start_line: name_node.start_position().row as u32,
        end_line: name_node.end_position().row as u32,
        start_col: name_node.start_position().column as u32,
        end_col: name_node.end_position().column as u32,
        signature: None,
        doc_comment: None,
        scope_path,
        parent_index,
    });

    // Emit TypeRef if the catch variable has a type annotation.
    // Annotation may live on the `catch_parameter` node or directly on `param`.
    let annotation_parent = if param.kind() == "identifier" { node } else { &param };
    let mut acursor = annotation_parent.walk();
    for child in annotation_parent.children(&mut acursor) {
        if child.kind() == "type_annotation" {
            extract_type_ref_from_annotation(&child, src, idx, refs);
            break;
        }
    }
}

/// Extract the loop variable from a `for_in_statement` as a Variable symbol.
///
/// Handles `for (const item of items)` and `for (const key in obj)`:
/// - tree-sitter represents these as `for_in_statement` with `left` (the variable
///   declaration) and `right` (the iterable expression).
/// - We extract the variable name from `left` and build a chain from `right`
///   so the index builder can infer the element type from the iterable.
pub(super) fn extract_for_loop_var(
    node: &Node,
    src: &[u8],
    scope_tree: &crate::parser::scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let left = match node.child_by_field_name("left") {
        Some(n) => n,
        None => return,
    };
    let right = match node.child_by_field_name("right") {
        Some(n) => n,
        None => return,
    };

    // `left` is typically a `lexical_declaration` (`const item`) or an `identifier`.
    // Dig down to find the identifier.
    let name = if left.kind() == "identifier" {
        node_text(left, src)
    } else {
        // Look for a variable_declarator → identifier inside a lexical_declaration.
        let mut found = String::new();
        let mut cur = left.walk();
        'outer: for child in left.children(&mut cur) {
            if child.kind() == "variable_declarator" {
                if let Some(name_node) = child.child_by_field_name("name") {
                    if name_node.kind() == "identifier" {
                        found = node_text(name_node, src);
                        break 'outer;
                    }
                }
            } else if child.kind() == "identifier" {
                found = node_text(child, src);
                break 'outer;
            }
        }
        found
    };

    if name.is_empty() {
        return;
    }

    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let qualified_name = scope_tree::qualify(&name, parent_scope);
    let scope_path = scope_tree::scope_path(parent_scope);

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Variable,
        visibility: None,
        start_line: left.start_position().row as u32,
        end_line: left.end_position().row as u32,
        start_col: left.start_position().column as u32,
        end_col: left.end_position().column as u32,
        signature: Some(format!("const {name}")),
        doc_comment: None,
        scope_path,
        parent_index,
    });

    // Build a chain from the iterable (right side) so the index builder can
    // infer the element type.  For `for (const item of this.repo.findAll())`,
    // the chain is [this, repo, findAll] and the type engine unwraps the array.
    let iterable_node = if right.kind() == "await_expression" {
        right
            .child_by_field_name("value")
            .or_else(|| right.named_child(0))
            .unwrap_or(right)
    } else {
        right
    };

    let chain_source = if iterable_node.kind() == "call_expression" {
        iterable_node
            .child_by_field_name("function")
            .unwrap_or(iterable_node)
    } else {
        iterable_node
    };

    if let Some(chain) = build_chain(chain_source, src) {
        let target = chain.segments.last().map(|s| s.name.clone()).unwrap_or_default();
        if !target.is_empty() {
            refs.push(ExtractedRef {
                source_symbol_index: idx,
                target_name: target,
                kind: EdgeKind::TypeRef,
                line: right.start_position().row as u32,
                module: None,
                chain: Some(chain),
            });
        }
    } else if iterable_node.kind() == "identifier" {
        // Simple identifier iterable: `for (const item of items)` — emit a
        // plain TypeRef so the index builder can look up `items` type.
        let target = node_text(iterable_node, src);
        if !target.is_empty() {
            refs.push(ExtractedRef {
                source_symbol_index: idx,
                target_name: target,
                kind: EdgeKind::TypeRef,
                line: right.start_position().row as u32,
                module: None,
                chain: None,
            });
        }
    }
}
