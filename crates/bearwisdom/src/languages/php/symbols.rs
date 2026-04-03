// =============================================================================
// php/symbols.rs  —  Symbol extractors for PHP
// =============================================================================

use super::calls::{extract_calls_from_body, extract_trait_use};
use super::helpers::{
    build_class_signature, build_method_signature, extract_visibility, node_text, qualify,
    qualify_ns, scope_from_prefix,
};
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::Node;

pub(super) fn extract_namespace(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let name = node_text(&name_node, src);
    let qualified_name = qualify_ns(&name, qualified_prefix);
    let new_prefix = qualified_name.clone();
    let ns_prefix = name.replace('\\', ".");

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Namespace,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("namespace {name}")),
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });

    if let Some(body) = node.child_by_field_name("body") {
        super::extract::extract_from_node(body, src, symbols, refs, Some(idx), &new_prefix, &ns_prefix);
    }
}

pub(super) fn extract_class(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    namespace_prefix: &str,
    kind: SymbolKind,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let name = node_text(&name_node, src);
    let qualified_name = qualify(&name, qualified_prefix);
    let new_prefix = qualified_name.clone();

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: build_class_signature(node, src, &name, kind),
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });

    // Scan children for inheritance/implements (tree-sitter-php 0.24 unnamed children)
    let mut cc = node.walk();
    for child in node.children(&mut cc) {
        match child.kind() {
            "base_clause" => {
                let mut bc = child.walk();
                for base_child in child.children(&mut bc) {
                    if base_child.kind() == "qualified_name"
                        || base_child.kind() == "name"
                        || base_child.kind() == "identifier"
                    {
                        refs.push(ExtractedRef {
                            source_symbol_index: idx,
                            target_name: node_text(&base_child, src),
                            kind: EdgeKind::Inherits,
                            line: base_child.start_position().row as u32,
                            module: None,
                            chain: None,
                        });
                    }
                }
            }
            "class_interface_clause" => {
                extract_interface_list(&child, src, refs, idx, EdgeKind::Implements);
            }
            _ => {}
        }
    }

    // Legacy field-based fallback for older grammar versions
    if refs.iter().all(|r| r.source_symbol_index != idx || r.kind != EdgeKind::Inherits) {
        if let Some(base) = node.child_by_field_name("base_clause") {
            let mut c = base.walk();
            for bc in base.children(&mut c) {
                if bc.kind() == "qualified_name" || bc.kind() == "name" || bc.kind() == "identifier" {
                    refs.push(ExtractedRef {
                        source_symbol_index: idx,
                        target_name: node_text(&bc, src),
                        kind: EdgeKind::Inherits,
                        line: bc.start_position().row as u32,
                        module: None,
                        chain: None,
                    });
                }
            }
        }
    }
    if let Some(impls) = node.child_by_field_name("class_implements") {
        extract_interface_list(&impls, src, refs, idx, EdgeKind::Implements);
    }

    // Recurse into body
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "declaration_list" {
            extract_class_body(&child, src, symbols, refs, Some(idx), &new_prefix, namespace_prefix);
        }
    }
}

pub(super) fn extract_interface_list(
    node: &Node,
    src: &[u8],
    refs: &mut Vec<ExtractedRef>,
    class_idx: usize,
    edge_kind: EdgeKind,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "qualified_name" || child.kind() == "name" || child.kind() == "identifier" {
            refs.push(ExtractedRef {
                source_symbol_index: class_idx,
                target_name: node_text(&child, src),
                kind: edge_kind,
                line: child.start_position().row as u32,
                module: None,
                chain: None,
            });
        } else {
            extract_interface_list(&child, src, refs, class_idx, edge_kind);
        }
    }
}

pub(super) fn extract_class_body(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    namespace_prefix: &str,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "method_declaration" => {
                extract_method(&child, src, symbols, refs, parent_index, qualified_prefix);
            }
            "property_declaration" => {
                extract_property_declaration(&child, src, symbols, parent_index, qualified_prefix);
            }
            "use_declaration" => {
                extract_trait_use(&child, src, refs, symbols.len());
            }
            "const_declaration" => {
                extract_const_declaration(&child, src, symbols, parent_index, qualified_prefix);
            }
            "enum_declaration" => {
                extract_enum(&child, src, symbols, refs, parent_index, qualified_prefix, namespace_prefix);
            }
            _ => {}
        }
    }
}

pub(super) fn extract_method(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let name = node_text(&name_node, src);
    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = extract_visibility(node, src);

    let kind = if name == "__construct" {
        SymbolKind::Constructor
    } else {
        SymbolKind::Method
    };

    let signature = build_method_signature(node, src, &name);

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name,
        qualified_name,
        kind,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature,
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });

    super::decorators::extract_decorators(node, src, idx, refs);

    // Extract TypeRefs from typed parameters (non-promoted).
    if let Some(params) = node.child_by_field_name("parameters") {
        extract_param_type_refs(&params, src, refs, idx);
    }

    // PHP 8.0 constructor promotion: `public function __construct(public string $name)`.
    // Promoted params live in the `parameters` child of the method declaration.
    if let Some(params) = node.child_by_field_name("parameters") {
        extract_promoted_params(&params, src, symbols, refs, Some(idx), qualified_prefix);
    }

    if let Some(body) = node.child_by_field_name("body") {
        extract_calls_from_body(&body, src, idx, refs);
    }
}

/// Extract `property_promotion_parameter` nodes from a constructor parameter list.
///
/// PHP 8.0+: `public string $name` in function signature → Property symbol + TypeRef.
fn extract_promoted_params(
    params_node: &Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    use super::calls::extract_type_refs_from_php_type;

    let mut cursor = params_node.walk();
    for child in params_node.children(&mut cursor) {
        if child.kind() != "property_promotion_parameter" {
            continue;
        }

        // Visibility modifier: `public`, `protected`, `private`.
        let visibility = extract_visibility(&child, src);

        // Type hint: optional named child before the variable.
        let type_node_opt = child.child_by_field_name("type");

        // Variable name: `$name`.
        let var_node = match child.child_by_field_name("name") {
            Some(n) => n,
            None => {
                // Fallback: find a variable_name child.
                let mut cc = child.walk();
                let found = child.children(&mut cc).find(|c| c.kind() == "variable_name");
                match found {
                    Some(n) => n,
                    None => continue,
                }
            }
        };

        let raw = node_text(&var_node, src);
        let name = raw.trim_start_matches('$').to_string();
        if name.is_empty() || name == "this" {
            continue;
        }

        let qualified_name = qualify(&name, qualified_prefix);
        let prop_idx = symbols.len();

        symbols.push(ExtractedSymbol {
            name: name.clone(),
            qualified_name,
            kind: SymbolKind::Property,
            visibility,
            start_line: var_node.start_position().row as u32,
            end_line: child.end_position().row as u32,
            start_col: var_node.start_position().column as u32,
            end_col: child.end_position().column as u32,
            signature: None,
            doc_comment: None,
            scope_path: scope_from_prefix(qualified_prefix),
            parent_index,
        });

        if let Some(type_node) = type_node_opt {
            extract_type_refs_from_php_type(&type_node, src, refs, prop_idx);
        }
    }
}

/// Extract TypeRef edges from function/method parameter type hints.
///
/// Handles `simple_parameter` (`string $name`) and `variadic_parameter` (`...$items`).
pub(super) fn extract_param_type_refs(
    params_node: &Node,
    src: &[u8],
    refs: &mut Vec<ExtractedRef>,
    source_symbol_index: usize,
) {
    use super::calls::extract_type_refs_from_php_type;

    let mut cursor = params_node.walk();
    for child in params_node.children(&mut cursor) {
        match child.kind() {
            "simple_parameter" | "variadic_parameter" => {
                if let Some(type_node) = child.child_by_field_name("type") {
                    extract_type_refs_from_php_type(&type_node, src, refs, source_symbol_index);
                }
            }
            _ => {}
        }
    }
}

pub(super) fn extract_function(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    _namespace_prefix: &str,
    _inside_class: bool,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let name = node_text(&name_node, src);
    let qualified_name = qualify(&name, qualified_prefix);
    let signature = build_method_signature(node, src, &name);

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature,
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });

    // Extract TypeRefs from typed parameters.
    if let Some(params) = node.child_by_field_name("parameters") {
        extract_param_type_refs(&params, src, refs, idx);
    }

    if let Some(body) = node.child_by_field_name("body") {
        extract_calls_from_body(&body, src, idx, refs);
    }
}

pub(super) fn extract_property_declaration(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let visibility = extract_visibility(node, src);

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "property_element" {
            let mut vc = child.walk();
            for var in child.children(&mut vc) {
                if var.kind() == "variable_name" || var.kind() == "$variable_name" {
                    let raw = node_text(&var, src);
                    let name = raw.trim_start_matches('$').to_string();
                    let qualified_name = qualify(&name, qualified_prefix);
                    symbols.push(ExtractedSymbol {
                        name,
                        qualified_name,
                        kind: SymbolKind::Property,
                        visibility,
                        start_line: var.start_position().row as u32,
                        end_line: node.end_position().row as u32,
                        start_col: var.start_position().column as u32,
                        end_col: node.end_position().column as u32,
                        signature: None,
                        doc_comment: None,
                        scope_path: scope_from_prefix(qualified_prefix),
                        parent_index,
                    });
                    break;
                }
            }
        }
    }
}

pub(super) fn extract_const_declaration(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "const_element" {
            // Try `name` field first; fall back to first `name`-kind node.
            let name_node_opt = child.child_by_field_name("name").or_else(|| {
                // Collect to avoid cursor lifetime issue.
                let children: Vec<_> = {
                    let mut cc = child.walk();
                    child.children(&mut cc).collect()
                };
                children.into_iter().find(|c| c.kind() == "name")
            });
            if let Some(name_node) = name_node_opt {
                let name = node_text(&name_node, src);
                if name.is_empty() {
                    continue;
                }
                let qualified_name = qualify(&name, qualified_prefix);
                symbols.push(ExtractedSymbol {
                    name,
                    qualified_name,
                    kind: SymbolKind::Field,
                    visibility: Some(Visibility::Public),
                    start_line: child.start_position().row as u32,
                    end_line: child.end_position().row as u32,
                    start_col: child.start_position().column as u32,
                    end_col: child.end_position().column as u32,
                    signature: None,
                    doc_comment: None,
                    scope_path: scope_from_prefix(qualified_prefix),
                    parent_index,
                });
            }
        }
    }
}

/// Extract Variable symbols from `global $var;` or `static $cache = [];`.
pub(super) fn extract_global_static_vars(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    is_static: bool,
) {
    let sig_prefix = if is_static { "static" } else { "global" };
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        // `global $var` — direct variable_name children.
        if child.kind() == "variable_name" {
            let raw = node_text(&child, src);
            let name = raw.trim_start_matches('$').to_string();
            if !name.is_empty() && name != "this" {
                symbols.push(ExtractedSymbol {
                    name: name.clone(),
                    qualified_name: qualify(&name, qualified_prefix),
                    kind: SymbolKind::Variable,
                    visibility: Some(Visibility::Public),
                    start_line: child.start_position().row as u32,
                    end_line: node.end_position().row as u32,
                    start_col: child.start_position().column as u32,
                    end_col: node.end_position().column as u32,
                    signature: Some(format!("{sig_prefix} ${name}")),
                    doc_comment: None,
                    scope_path: scope_from_prefix(qualified_prefix),
                    parent_index,
                });
            }
        }
        // `static $cache = []` — static_variable_declaration wraps a `variable_name`.
        if child.kind() == "static_variable_declarator" {
            if let Some(name_node) = child.child_by_field_name("name") {
                let raw = node_text(&name_node, src);
                let name = raw.trim_start_matches('$').to_string();
                if !name.is_empty() {
                    symbols.push(ExtractedSymbol {
                        name: name.clone(),
                        qualified_name: qualify(&name, qualified_prefix),
                        kind: SymbolKind::Variable,
                        visibility: Some(Visibility::Public),
                        start_line: name_node.start_position().row as u32,
                        end_line: child.end_position().row as u32,
                        start_col: name_node.start_position().column as u32,
                        end_col: child.end_position().column as u32,
                        signature: Some(format!("{sig_prefix} ${name}")),
                        doc_comment: None,
                        scope_path: scope_from_prefix(qualified_prefix),
                        parent_index,
                    });
                }
            }
        }
    }
}

/// Handle an `expression_statement` at module/function top level.
///
/// Looks for:
/// - `list($a, $b) = ...` / `[$a, $b] = ...` — array destructuring.
/// - Any other expression with calls — delegate to `extract_calls_from_body`.
pub(super) fn extract_expression_statement(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let source_idx = parent_index.unwrap_or(0);
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // `$a = $b` / `[$x, $y] = expr` / `list($x) = expr`
            "assignment_expression" => {
                if let Some(left) = child.child_by_field_name("left") {
                    match left.kind() {
                        "array_creation_expression" | "list_literal" => {
                            super::calls::extract_list_destructuring(
                                &left,
                                src,
                                symbols,
                                parent_index,
                                qualified_prefix,
                            );
                        }
                        _ => {}
                    }
                }
                // Always extract calls from the RHS.
                extract_calls_from_body(&child, src, source_idx, refs);
            }
            // `include 'file.php'` / `require_once 'config.php'` at statement level.
            "include_expression"
            | "include_once_expression"
            | "require_expression"
            | "require_once_expression" => {
                super::calls::extract_include_require(&child, src, refs, source_idx);
            }
            // Direct call-site nodes: pass the whole statement so that
            // `extract_calls_from_body` sees the call node as a child.
            "function_call_expression"
            | "member_call_expression"
            | "nullsafe_member_call_expression"
            | "static_call_expression"
            | "object_creation_expression" => {
                // Pass the expression_statement as the parent so the call node
                // is seen as a child and matched by the extractor's match arms.
                extract_calls_from_body(node, src, source_idx, refs);
                break; // one call per expression_statement is sufficient
            }
            _ => {
                extract_calls_from_body(&child, src, source_idx, refs);
            }
        }
    }
}

pub(super) fn extract_enum(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    _namespace_prefix: &str,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let name = node_text(&name_node, src);
    let qualified_name = qualify(&name, qualified_prefix);
    let new_prefix = qualified_name.clone();

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Enum,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("enum {name}")),
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });

    if let Some(impls) = node.child_by_field_name("class_implements") {
        extract_interface_list(&impls, src, refs, idx, EdgeKind::Implements);
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "enum_declaration_list" {
            let mut lc = child.walk();
            for item in child.children(&mut lc) {
                match item.kind() {
                    "enum_case" => {
                        if let Some(nm) = item.child_by_field_name("name") {
                            let case_name = node_text(&nm, src);
                            let case_qn = qualify(&case_name, &new_prefix);
                            symbols.push(ExtractedSymbol {
                                name: case_name,
                                qualified_name: case_qn,
                                kind: SymbolKind::EnumMember,
                                visibility: Some(Visibility::Public),
                                start_line: item.start_position().row as u32,
                                end_line: item.end_position().row as u32,
                                start_col: item.start_position().column as u32,
                                end_col: item.end_position().column as u32,
                                signature: None,
                                doc_comment: None,
                                scope_path: Some(new_prefix.clone()),
                                parent_index: Some(idx),
                            });
                        }
                    }
                    "method_declaration" => {
                        extract_method(&item, src, symbols, refs, Some(idx), &new_prefix);
                    }
                    _ => {}
                }
            }
        }
    }
}
