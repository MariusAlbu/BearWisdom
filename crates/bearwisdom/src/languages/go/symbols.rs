// =============================================================================
// go/symbols.rs  —  Symbol extraction for Go declarations
// =============================================================================

use super::calls::extract_body_with_symbols;
use super::helpers::{
    build_fn_signature_from_source, extract_go_doc_comment, extract_go_type_name, go_visibility,
    is_go_builtin_type, is_test_function, node_text, pointer_type_name, qualify,
    scope_from_prefix,
};
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// Package clause
// ---------------------------------------------------------------------------

/// Emit a `Namespace` symbol for the `package_clause` node.
pub(super) fn extract_package_clause(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    qualified_prefix: &str,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "package_identifier" {
            let name = node_text(&child, source);
            if name.is_empty() {
                return;
            }
            let qname = qualify(&name, qualified_prefix);
            symbols.push(ExtractedSymbol {
                name: name.clone(),
                qualified_name: qname,
                kind: SymbolKind::Namespace,
                visibility: Some(Visibility::Public),
                start_line: node.start_position().row as u32,
                end_line: node.end_position().row as u32,
                start_col: node.start_position().column as u32,
                end_col: node.end_position().column as u32,
                signature: Some(format!("package {name}")),
                doc_comment: None,
                scope_path: None,
                parent_index: None,
                byte_offset: 0,
                            declared_type: None,
                return_type: None,
                param_types: Vec::new(),
                generic_params: Vec::new(),
});
            return;
        }
    }
}

// ---------------------------------------------------------------------------
// Import declarations
// ---------------------------------------------------------------------------

pub(super) fn extract_import_declaration(
    node: &Node,
    source: &str,
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_spec" => {
                emit_import_ref(&child, source, refs, current_symbol_count);
            }
            "import_spec_list" => {
                let mut inner = child.walk();
                for spec in child.children(&mut inner) {
                    if spec.kind() == "import_spec" {
                        emit_import_ref(&spec, source, refs, current_symbol_count);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Emit one `Imports` ref for an `import_spec` node.
///
/// `import_spec` children (positional):
///   [dot | blank_identifier | package_identifier]  (optional alias)
///   interpreted_string_literal  (the import path)
fn emit_import_ref(
    node: &Node,
    source: &str,
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
) {
    // Find the string literal child — it is the last named child.
    let mut path_text: Option<String> = None;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "interpreted_string_literal"
            || child.kind() == "raw_string_literal"
        {
            path_text = Some(node_text(&child, source));
        }
    }

    let raw = match path_text {
        Some(s) => s,
        None => return,
    };

    // Strip surrounding quotes / backticks.
    let full_path = raw.trim_matches('"').trim_matches('`');

    let target_name = full_path
        .rsplit('/')
        .next()
        .unwrap_or(full_path)
        .to_string();

    let module = if full_path.is_empty() {
        None
    } else {
        Some(full_path.to_string())
    };

    refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
        source_symbol_index: current_symbol_count,
        target_name,
        kind: EdgeKind::Imports,
        line: node.start_position().row as u32,
        col: 0,
        module,
        chain: None,
        byte_offset: node.start_byte() as u32,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
});
}

// ---------------------------------------------------------------------------
// Function declarations
// ---------------------------------------------------------------------------

/// `function_declaration` children (positional, named only):
///   identifier (name), parameter_list (params), result?, block (body)
///
/// The `func` keyword child is unnamed; we skip it via `is_named()`.
pub(super) fn extract_function_declaration(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    // First named child whose kind is `identifier` is the function name.
    let (name, params_opt, body_opt) = parse_function_decl_children(node, source);
    let name = match name {
        Some(n) => n,
        None => return,
    };

    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = go_visibility(&name);
    let doc_comment = extract_go_doc_comment(node, source);
    let signature = build_fn_signature_from_source(node, source);

    let kind = if is_test_function(&name) {
        SymbolKind::Test
    } else {
        SymbolKind::Function
    };

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name,
        qualified_name: qualified_name.clone(),
        kind,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature,
        doc_comment,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
            byte_offset: 0,
    declared_type: None,
    return_type: None,
    param_types: Vec::new(),
    generic_params: Vec::new(),
});

    // Extract TypeRef edges from parameter and return types.
    super::calls::extract_fn_signature_type_refs(node, source, idx, refs);

    // Extract typed parameters as Property symbols scoped to this function.
    if let Some(params) = params_opt {
        extract_go_typed_params_as_symbols(&params, source, symbols, refs, Some(idx), &qualified_name);
    }

    if let Some(body) = body_opt {
        extract_body_with_symbols(&body, source, idx, &qualified_name, symbols, refs);
    }
}

/// Returns (name, params_node, body_node) from a `function_declaration`.
fn parse_function_decl_children<'a>(
    node: &'a Node<'a>,
    source: &str,
) -> (Option<String>, Option<Node<'a>>, Option<Node<'a>>) {
    let mut name: Option<String> = None;
    let mut params: Option<Node<'a>> = None;
    let mut body: Option<Node<'a>> = None;
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if !child.is_named() {
            continue;
        }
        match child.kind() {
            "identifier" if name.is_none() => {
                name = Some(node_text(&child, source));
            }
            "parameter_list" if params.is_none() => {
                // The first (and only) parameter_list in a function_declaration
                // is the regular parameter list.
                params = Some(child);
            }
            "block" => {
                body = Some(child);
            }
            _ => {}
        }
    }

    (name, params, body)
}

// ---------------------------------------------------------------------------
// Method declarations
// ---------------------------------------------------------------------------

/// `method_declaration` children (positional, named only):
///   parameter_list (receiver), field_identifier (name), parameter_list (params),
///   result?, block (body)
pub(super) fn extract_method_declaration(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let (receiver_type, name, params_opt, body_opt) = parse_method_decl_children(node, source);

    let name = match name {
        Some(n) => n,
        None => return,
    };

    // Qualified name: <package>.<ReceiverType>.<MethodName>
    let method_prefix = match &receiver_type {
        Some(rt) => qualify(rt, qualified_prefix),
        None => qualified_prefix.to_string(),
    };

    let qualified_name = qualify(&name, &method_prefix);
    let visibility = go_visibility(&name);
    let doc_comment = extract_go_doc_comment(node, source);
    let signature = build_fn_signature_from_source(node, source);

    let kind = if is_test_function(&name) {
        SymbolKind::Test
    } else {
        SymbolKind::Method
    };

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name,
        qualified_name: qualified_name.clone(),
        kind,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature,
        doc_comment,
        scope_path: scope_from_prefix(&method_prefix),
        parent_index,
            byte_offset: 0,
    declared_type: None,
    return_type: None,
    param_types: Vec::new(),
    generic_params: Vec::new(),
});

    // Extract TypeRef edges from parameter and return types.
    super::calls::extract_fn_signature_type_refs(node, source, idx, refs);

    // Extract typed parameters as Property symbols scoped to this method.
    if let Some(params) = params_opt {
        extract_go_typed_params_as_symbols(&params, source, symbols, refs, Some(idx), &qualified_name);
    }

    if let Some(body) = body_opt {
        extract_body_with_symbols(&body, source, idx, &qualified_name, symbols, refs);
    }
}

/// Parse the children of a `method_declaration` and return
/// `(receiver_type, method_name, params_node, body)`.
///
/// Child order: `func` (anon), parameter_list (receiver), field_identifier (name),
/// parameter_list (params), result?, block (body)
fn parse_method_decl_children<'a>(
    node: &'a Node<'a>,
    source: &str,
) -> (Option<String>, Option<String>, Option<Node<'a>>, Option<Node<'a>>) {
    let mut receiver_type: Option<String> = None;
    let mut method_name: Option<String> = None;
    let mut params: Option<Node<'a>> = None;
    let mut body: Option<Node<'a>> = None;
    let mut param_list_count = 0usize;

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if !child.is_named() {
            continue; // skip `func`, `(`, `)`, etc.
        }
        match child.kind() {
            "parameter_list" => {
                param_list_count += 1;
                if param_list_count == 1 {
                    // First parameter_list is the receiver `(p Point)`.
                    receiver_type = extract_receiver_type_from_param_list(&child, source);
                } else if param_list_count == 2 {
                    // Second parameter_list is the regular parameters.
                    params = Some(child);
                }
            }
            "field_identifier" => {
                // Method name.
                if method_name.is_none() {
                    method_name = Some(node_text(&child, source));
                }
            }
            "block" => {
                body = Some(child);
            }
            _ => {}
        }
    }

    (receiver_type, method_name, params, body)
}

/// Extract the plain type name from a receiver `parameter_list`.
///
/// `(p Point)` or `(s *Server)` → `"Point"` / `"Server"`.
fn extract_receiver_type_from_param_list(param_list: &Node, source: &str) -> Option<String> {
    let mut cursor = param_list.walk();
    for child in param_list.children(&mut cursor) {
        if child.kind() == "parameter_declaration" {
            // parameter_declaration children (positional):
            //   identifier (receiver var name), type
            // The type is the last named child.
            let mut ccursor = child.walk();
            let mut type_text: Option<String> = None;
            for cc in child.children(&mut ccursor) {
                if !cc.is_named() {
                    continue;
                }
                match cc.kind() {
                    // Direct type_identifier → `Point`
                    "type_identifier" => {
                        type_text = Some(node_text(&cc, source));
                    }
                    // `*Server` → pointer_type
                    "pointer_type" => {
                        // Strip the `*` — just find the inner type_identifier.
                        type_text = Some(pointer_type_name(&cc, source));
                    }
                    _ => {}
                }
            }
            return type_text;
        }
    }
    None
}
// ---------------------------------------------------------------------------
// Typed parameter symbol extraction
// ---------------------------------------------------------------------------

/// Extract typed parameters from a Go `parameter_list` as Property symbols
/// scoped to the enclosing function or method.
///
/// For `func GetUser(repo UserRepository, id int)`, creates:
///   Symbol: `mypackage.GetUser.repo` (kind=Property)
///   TypeRef: `mypackage.GetUser.repo → UserRepository`
///
/// Skips parameters without names (bare type declarations in interfaces) and
/// parameters with only builtin types since they don't reference user symbols.
///
/// Go `parameter_declaration` structure:
///   `commaSep(field('name', identifier))`, `field('type', _type)`
pub(super) fn extract_go_typed_params_as_symbols(
    params_node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    func_qualified_name: &str,
) {
    let mut cursor = params_node.walk();
    for child in params_node.children(&mut cursor) {
        // `variadic_parameter_declaration` has the same field layout as
        // `parameter_declaration` — the type field holds the element type
        // (without the `...`).  Treat them identically.
        if child.kind() != "parameter_declaration"
            && child.kind() != "variadic_parameter_declaration"
        {
            continue;
        }

        // Collect all `name` field nodes (Go allows `a, b int`).
        let names: Vec<String> = (0..child.child_count())
            .filter_map(|i| child.child(i))
            .filter(|c| c.is_named() && c.kind() == "identifier")
            .map(|c| node_text(&c, source))
            .collect();

        if names.is_empty() {
            // No name — bare type in interface method or unnamed param.
            continue;
        }

        // The type is the last named child that isn't an identifier.
        let type_node = (0..child.child_count())
            .filter_map(|i| child.child(i))
            .filter(|c| c.is_named() && c.kind() != "identifier")
            .last();

        let type_name = match type_node {
            Some(tn) => extract_go_type_name(&tn, source),
            None => continue,
        };

        if type_name.is_empty() || is_go_builtin_type(&type_name) {
            continue;
        }

        for name in names {
            let qualified_name = qualify(&name, func_qualified_name);
            let scope_path = Some(func_qualified_name.to_string());

            let param_idx = symbols.len();
            symbols.push(ExtractedSymbol {
                name: name.clone(),
                qualified_name,
                kind: SymbolKind::Property,
                visibility: None,
                start_line: child.start_position().row as u32,
                end_line: child.end_position().row as u32,
                start_col: child.start_position().column as u32,
                end_col: child.end_position().column as u32,
                signature: Some(format!("{name} {type_name}")),
                doc_comment: None,
                scope_path,
                parent_index,
                            byte_offset: 0,
    declared_type: None,
    return_type: None,
    param_types: Vec::new(),
    generic_params: Vec::new(),
});

            refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                source_symbol_index: param_idx,
                target_name: type_name.clone(),
                kind: EdgeKind::TypeRef,
                line: child.start_position().row as u32,
                col: 0,
                module: None,
                chain: None,
                byte_offset: child.start_byte() as u32,
                            namespace_segments: Vec::new(),
                            call_args: Vec::new(),
});
        }
    }
}
