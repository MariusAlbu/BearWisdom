// =============================================================================
// parser/extractors/go.rs  —  Go symbol and reference extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Namespace  — package name (used as qualified-name prefix)
//   Struct     — `type Foo struct { ... }`
//   Interface  — `type Foo interface { ... }`
//   TypeAlias  — `type Foo Bar` / `type Foo = Bar` (non-struct/interface)
//   Function   — top-level `func Foo(...)`
//   Method     — `func (r ReceiverType) MethodName(...)` → `ReceiverType.MethodName`
//   Method     — interface method element signatures (`method_elem`)
//   Field      — struct fields
//   Variable   — `const` and `var` declarations
//   Test       — functions named Test*, Benchmark*, or Example*
//
// REFERENCES:
//   import_declaration / import_spec → EdgeKind::Imports
//   call_expression                  → EdgeKind::Calls
//   composite_literal                → EdgeKind::Instantiates
//   embedded struct fields           → EdgeKind::Inherits
//
// Visibility:
//   Go has no explicit modifier — exported names start with a Unicode uppercase
//   letter.  Unexported names → Private.
//
// Grammar notes (tree-sitter-go):
//   This grammar exposes almost everything via positional children rather than
//   named fields.  The few named fields that do exist:
//     function_declaration  → no named fields; children in order:
//                             `func`, identifier (name), parameter_list (params),
//                             result?, block (body)
//     method_declaration    → children: `func`, parameter_list (receiver),
//                             field_identifier (name), parameter_list (params),
//                             result?, block (body)
//     type_spec             → children: type_identifier (name), [=], type body
//     struct_type           → child: field_declaration_list
//     field_declaration_list→ children: field_declaration*
//     field_declaration     → children: field_identifier* (names), type
//                             OR just: type_identifier (embedded)
//     interface_type        → children: method_elem*  (NOT method_spec)
//     method_elem           → children: field_identifier, parameter_list (params), result?
//     package_clause        → children: `package`, package_identifier
//     import_spec           → children: [dot|blank_identifier|package_identifier], string
//     const_spec / var_spec → children: identifier* (names), [type], [= values]
// =============================================================================

use crate::types::{
    ChainSegment, EdgeKind, ExtractedRef, ExtractedSymbol, MemberChain, SegmentKind, SymbolKind,
    Visibility,
};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub struct GoExtraction {
    pub symbols: Vec<ExtractedSymbol>,
    pub refs: Vec<ExtractedRef>,
    pub has_errors: bool,
}

/// Extract all symbols and references from Go source code.
pub fn extract(source: &str) -> GoExtraction {
    let language = tree_sitter_go::LANGUAGE.into();

    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to set Go grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => {
            return GoExtraction {
                symbols: vec![],
                refs: vec![],
                has_errors: true,
            }
        }
    };

    let root = tree.root_node();

    // Hoist the package name so it becomes the qualified-name prefix for all
    // top-level symbols.
    let package_name = hoist_package_name(root, source);
    let qualified_prefix = package_name.as_deref().unwrap_or("");

    let mut symbols = Vec::new();
    let mut refs = Vec::new();

    extract_from_node(root, source, &mut symbols, &mut refs, None, qualified_prefix);

    let has_errors = root.has_error();
    GoExtraction { symbols, refs, has_errors }
}

// ---------------------------------------------------------------------------
// Package name hoist
// ---------------------------------------------------------------------------

/// Find the `package_clause` and return the package identifier text.
///
/// `package_clause` children: `package` (keyword), `package_identifier`.
fn hoist_package_name(root: Node, source: &str) -> Option<String> {
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "package_clause" {
            let mut cc = child.walk();
            for inner in child.children(&mut cc) {
                if inner.kind() == "package_identifier" {
                    return Some(node_text(&inner, source));
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Core traversal
// ---------------------------------------------------------------------------

fn extract_from_node(
    node: Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            // Already handled in hoist pass.
            "package_clause" => {}

            "import_declaration" => {
                extract_import_declaration(&child, source, refs, symbols.len());
            }

            "function_declaration" => {
                extract_function_declaration(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                );
            }

            "method_declaration" => {
                extract_method_declaration(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                );
            }

            "type_declaration" => {
                extract_type_declaration(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                );
            }

            "const_declaration" => {
                extract_const_var_decl(
                    &child,
                    source,
                    symbols,
                    parent_index,
                    qualified_prefix,
                    "const",
                    "const_spec",
                );
            }

            "var_declaration" => {
                extract_const_var_decl(
                    &child,
                    source,
                    symbols,
                    parent_index,
                    qualified_prefix,
                    "var",
                    "var_spec",
                );
            }

            "ERROR" | "MISSING" => {}

            _ => {
                extract_from_node(child, source, symbols, refs, parent_index, qualified_prefix);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Import declarations
// ---------------------------------------------------------------------------

fn extract_import_declaration(
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

    refs.push(ExtractedRef {
        source_symbol_index: current_symbol_count,
        target_name,
        kind: EdgeKind::Imports,
        line: node.start_position().row as u32,
        module,
        chain: None,
    });
}

// ---------------------------------------------------------------------------
// Function declarations
// ---------------------------------------------------------------------------

/// `function_declaration` children (positional, named only):
///   identifier (name), parameter_list (params), result?, block (body)
///
/// The `func` keyword child is unnamed; we skip it via `is_named()`.
fn extract_function_declaration(
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
    });

    // Extract typed parameters as Property symbols scoped to this function.
    if let Some(params) = params_opt {
        extract_go_typed_params_as_symbols(&params, source, symbols, refs, Some(idx), &qualified_name);
    }

    if let Some(body) = body_opt {
        extract_refs_from_body(&body, source, idx, refs);
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
fn extract_method_declaration(
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
    });

    // Extract typed parameters as Property symbols scoped to this method.
    if let Some(params) = params_opt {
        extract_go_typed_params_as_symbols(&params, source, symbols, refs, Some(idx), &qualified_name);
    }

    if let Some(body) = body_opt {
        extract_refs_from_body(&body, source, idx, refs);
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

/// Extract the base type name from a `pointer_type` node (`*Foo` → `"Foo"`).
fn pointer_type_name(node: &Node, source: &str) -> String {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_identifier" {
            return node_text(&child, source);
        }
        if child.kind() == "pointer_type" {
            // Handle `**Foo`
            return pointer_type_name(&child, source);
        }
    }
    // Fallback: strip leading `*` from raw text.
    node_text(node, source).trim_start_matches('*').to_string()
}

// ---------------------------------------------------------------------------
// Type declarations
// ---------------------------------------------------------------------------

fn extract_type_declaration(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_spec" {
            extract_type_spec(
                &child,
                source,
                symbols,
                refs,
                parent_index,
                qualified_prefix,
            );
        }
    }
}

/// `type_spec` children (positional, named):
///   type_identifier (name), [=], struct_type | interface_type | other_type
fn extract_type_spec(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    // The first named child is always the type_identifier (name).
    // The second named child is the type body (may be struct_type, interface_type,
    // or any other type expression).
    let mut named_children: Vec<Node> = {
        let mut cursor = node.walk();
        node.children(&mut cursor)
            .filter(|c| c.is_named())
            .collect()
    };

    if named_children.is_empty() {
        return;
    }

    let name_node = named_children.remove(0);
    if name_node.kind() != "type_identifier" {
        return;
    }
    let name = node_text(&name_node, source);

    // `named_children` now holds [type_body] (after removing the name node).
    // For `type Foo = Bar` the `=` is an anonymous node so it doesn't appear
    // in named_children; the type body is still the first (and only) remaining.
    let type_node = match named_children.into_iter().next() {
        Some(n) => n,
        None => return,
    };

    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = go_visibility(&name);
    let doc_comment = extract_go_doc_comment(node, source);

    match type_node.kind() {
        "struct_type" => {
            let sig = format!("type {name} struct");
            let idx = symbols.len();
            let struct_prefix = qualify(&name, qualified_prefix);
            symbols.push(ExtractedSymbol {
                name: name.clone(),
                qualified_name,
                kind: SymbolKind::Struct,
                visibility,
                start_line: node.start_position().row as u32,
                end_line: node.end_position().row as u32,
                start_col: node.start_position().column as u32,
                end_col: node.end_position().column as u32,
                signature: Some(sig),
                doc_comment,
                scope_path: scope_from_prefix(qualified_prefix),
                parent_index,
            });
            extract_struct_fields(&type_node, source, symbols, refs, Some(idx), &struct_prefix);
        }

        "interface_type" => {
            let sig = format!("type {name} interface");
            let idx = symbols.len();
            let iface_prefix = qualify(&name, qualified_prefix);
            symbols.push(ExtractedSymbol {
                name: name.clone(),
                qualified_name,
                kind: SymbolKind::Interface,
                visibility,
                start_line: node.start_position().row as u32,
                end_line: node.end_position().row as u32,
                start_col: node.start_position().column as u32,
                end_col: node.end_position().column as u32,
                signature: Some(sig),
                doc_comment,
                scope_path: scope_from_prefix(qualified_prefix),
                parent_index,
            });
            extract_interface_methods(&type_node, source, symbols, Some(idx), &iface_prefix);
        }

        _ => {
            // Defined type or alias (`type Foo Bar` / `type Foo = Bar`).
            let type_text = node_text(&type_node, source);
            let sig = format!("type {name} {type_text}");
            symbols.push(ExtractedSymbol {
                name,
                qualified_name,
                kind: SymbolKind::TypeAlias,
                visibility,
                start_line: node.start_position().row as u32,
                end_line: node.end_position().row as u32,
                start_col: node.start_position().column as u32,
                end_col: node.end_position().column as u32,
                signature: Some(sig),
                doc_comment,
                scope_path: scope_from_prefix(qualified_prefix),
                parent_index,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Struct fields
// ---------------------------------------------------------------------------

/// Walk the `struct_type` → `field_declaration_list` and emit Field symbols.
///
/// Embedded (anonymous) fields also emit `Inherits` refs.
fn extract_struct_fields(
    struct_node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    struct_prefix: &str,
) {
    // struct_type children: `struct` (anon keyword), field_declaration_list
    let mut cursor = struct_node.walk();
    for child in struct_node.children(&mut cursor) {
        if child.kind() == "field_declaration_list" {
            extract_field_declaration_list(
                &child,
                source,
                symbols,
                refs,
                parent_index,
                struct_prefix,
            );
        }
    }
}

fn extract_field_declaration_list(
    list_node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    struct_prefix: &str,
) {
    let mut cursor = list_node.walk();
    for child in list_node.children(&mut cursor) {
        if child.kind() == "field_declaration" {
            extract_field_declaration(
                &child,
                source,
                symbols,
                refs,
                parent_index,
                struct_prefix,
            );
        }
    }
}

/// A `field_declaration` is one of:
///
///   Named:    `field_identifier+ type`   — one or more names, then a type
///   Embedded: `type_identifier`          — just the embedded type name
///   Embedded: `pointer_type`             — `*EmbeddedType`
///
/// We distinguish these by looking for `field_identifier` children.
fn extract_field_declaration(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    struct_prefix: &str,
) {
    let mut field_names: Vec<String> = Vec::new();
    let mut type_text: Option<String> = None;
    let mut embedded_type: Option<String> = None;

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if !child.is_named() {
            continue;
        }
        match child.kind() {
            "field_identifier" => {
                field_names.push(node_text(&child, source));
            }
            "type_identifier" if field_names.is_empty() && type_text.is_none() => {
                // The only named child — this is an embedded type.
                embedded_type = Some(node_text(&child, source));
            }
            "pointer_type" if field_names.is_empty() && type_text.is_none() => {
                // `*EmbeddedType`
                embedded_type = Some(pointer_type_name(&child, source));
            }
            _ => {
                // Any other named child after field_identifier(s) is the type.
                if !field_names.is_empty() || type_text.is_none() {
                    if !field_names.is_empty() {
                        type_text = Some(node_text(&child, source));
                    }
                }
            }
        }
    }

    if let Some(et) = embedded_type {
        if !et.is_empty() {
            // Emit Inherits edge from the struct (parent_index) to the embedded type.
            refs.push(ExtractedRef {
                source_symbol_index: parent_index.unwrap_or(0),
                target_name: et.clone(),
                kind: EdgeKind::Inherits,
                line: node.start_position().row as u32,
                module: None,
                chain: None,
            });
            // Also emit a Field symbol (the embedded type acts as an accessible field).
            symbols.push(ExtractedSymbol {
                name: et.clone(),
                qualified_name: qualify(&et, struct_prefix),
                kind: SymbolKind::Field,
                visibility: go_visibility(&et),
                start_line: node.start_position().row as u32,
                end_line: node.end_position().row as u32,
                start_col: node.start_position().column as u32,
                end_col: node.end_position().column as u32,
                signature: None,
                doc_comment: None,
                scope_path: scope_from_prefix(struct_prefix),
                parent_index,
            });
        }
    } else {
        // Named fields.
        let type_str = type_text.unwrap_or_default();
        for field_name in field_names {
            let vis = go_visibility(&field_name);
            let sig = if type_str.is_empty() {
                field_name.clone()
            } else {
                format!("{field_name} {type_str}")
            };
            symbols.push(ExtractedSymbol {
                name: field_name.clone(),
                qualified_name: qualify(&field_name, struct_prefix),
                kind: SymbolKind::Field,
                visibility: vis,
                start_line: node.start_position().row as u32,
                end_line: node.end_position().row as u32,
                start_col: node.start_position().column as u32,
                end_col: node.end_position().column as u32,
                signature: Some(sig),
                doc_comment: None,
                scope_path: scope_from_prefix(struct_prefix),
                parent_index,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Interface method elements
// ---------------------------------------------------------------------------

/// Walk the `interface_type` node and emit `Method` symbols for each
/// `method_elem`.
///
/// `method_elem` children: field_identifier (name), parameter_list (params),
/// result?
fn extract_interface_methods(
    iface_node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    iface_prefix: &str,
) {
    let mut cursor = iface_node.walk();
    for child in iface_node.children(&mut cursor) {
        if child.kind() != "method_elem" {
            continue;
        }

        // Find the field_identifier child by index (avoids cursor borrow issue).
        let name = (0..child.named_child_count())
            .filter_map(|i| child.named_child(i))
            .find(|c| c.kind() == "field_identifier")
            .map(|n| node_text(&n, source));

        let name = match name {
            Some(n) => n,
            None => continue,
        };

        let qualified_name = qualify(&name, iface_prefix);
        let visibility = go_visibility(&name);
        let signature = build_method_elem_signature(&child, source);

        symbols.push(ExtractedSymbol {
            name,
            qualified_name,
            kind: SymbolKind::Method,
            visibility,
            start_line: child.start_position().row as u32,
            end_line: child.end_position().row as u32,
            start_col: child.start_position().column as u32,
            end_col: child.end_position().column as u32,
            signature,
            doc_comment: None,
            scope_path: scope_from_prefix(iface_prefix),
            parent_index,
        });
    }
}

// ---------------------------------------------------------------------------
// Const / var declarations
// ---------------------------------------------------------------------------

fn extract_const_var_decl(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    keyword: &str,
    spec_kind: &str,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == spec_kind {
            extract_const_var_spec(
                &child,
                source,
                symbols,
                parent_index,
                qualified_prefix,
                keyword,
            );
        }
    }
}

/// `const_spec` / `var_spec` children:
///   identifier+ (names), [type], [= expression_list]
fn extract_const_var_spec(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    keyword: &str,
) {
    let mut names: Vec<(String, u32, u32)> = Vec::new();
    let mut type_text: Option<String> = None;
    let mut past_names = false;

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if !child.is_named() {
            // Skip `=` and `,`
            // But once we see `=` (anonymous) we know we're past the names.
            if node_text(&child, source) == "=" {
                past_names = true;
            }
            continue;
        }
        match child.kind() {
            "identifier" if !past_names => {
                names.push((
                    node_text(&child, source),
                    child.start_position().row as u32,
                    child.start_position().column as u32,
                ));
            }
            _ if !past_names && type_text.is_none() && !names.is_empty() => {
                type_text = Some(node_text(&child, source));
                past_names = true;
            }
            _ => {}
        }
    }

    for (name, start_line, start_col) in names {
        let qualified_name = qualify(&name, qualified_prefix);
        let visibility = go_visibility(&name);
        let sig = if let Some(ref t) = type_text {
            format!("{keyword} {name} {t}")
        } else {
            format!("{keyword} {name}")
        };

        symbols.push(ExtractedSymbol {
            name,
            qualified_name,
            kind: SymbolKind::Variable,
            visibility,
            start_line,
            end_line: node.end_position().row as u32,
            start_col,
            end_col: node.end_position().column as u32,
            signature: Some(sig),
            doc_comment: extract_go_doc_comment(node, source),
            scope_path: scope_from_prefix(qualified_prefix),
            parent_index,
        });
    }
}

// ---------------------------------------------------------------------------
// Body reference extraction (calls, instantiations)
// ---------------------------------------------------------------------------

fn extract_refs_from_body(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "call_expression" => {
                extract_call_ref(&child, source, source_symbol_index, refs);
                // Recurse into arguments for nested calls.
                let mut acursor = child.walk();
                for arg_child in child.children(&mut acursor) {
                    if arg_child.kind() == "argument_list" {
                        extract_refs_from_body(
                            &arg_child,
                            source,
                            source_symbol_index,
                            refs,
                        );
                    }
                }
            }
            "composite_literal" => {
                extract_composite_literal_ref(&child, source, source_symbol_index, refs);
                // Recurse into body for nested composites / calls.
                let mut bcursor = child.walk();
                for body_child in child.children(&mut bcursor) {
                    if body_child.kind() == "literal_value" {
                        extract_refs_from_body(
                            &body_child,
                            source,
                            source_symbol_index,
                            refs,
                        );
                    }
                }
            }
            _ => {
                extract_refs_from_body(&child, source, source_symbol_index, refs);
            }
        }
    }
}

/// Emit a `Calls` ref for a `call_expression`.
///
/// `call_expression` children (positional):
///   function (identifier | selector_expression | ...), argument_list
///
/// For `bar.Baz()` the function part is a `selector_expression` with children:
///   operand, `.`, `field_identifier`
fn extract_call_ref(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // The function part is the first named child (use index to avoid cursor borrow).
    let func_node = match node.named_child(0) {
        Some(n) => n,
        None => return,
    };

    // Build a structured chain for selector expressions; fall back to the
    // existing single-name extraction for bare identifiers.
    let chain = build_chain(func_node, source);

    let target_name = chain
        .as_ref()
        .and_then(|c| c.segments.last())
        .map(|s| s.name.clone())
        .unwrap_or_else(|| match func_node.kind() {
            "selector_expression" => (0..func_node.named_child_count())
                .filter_map(|i| func_node.named_child(i))
                .find(|c| c.kind() == "field_identifier")
                .map(|n| node_text(&n, source))
                .unwrap_or_else(|| node_text(&func_node, source)),
            _ => node_text(&func_node, source),
        });

    if target_name.is_empty() {
        return;
    }

    refs.push(ExtractedRef {
        source_symbol_index,
        target_name,
        kind: EdgeKind::Calls,
        line: func_node.start_position().row as u32,
        module: None,
        chain,
    });
}

// ---------------------------------------------------------------------------
// Member chain builder
// ---------------------------------------------------------------------------

/// Build a structured `MemberChain` from a Go function/selector node.
///
/// Go uses `selector_expression` for member access (not `member_expression`):
///
/// `repo.FindOne()`:
/// ```text
/// selector_expression
///   identifier "repo"
///   field_identifier "FindOne"
/// ```
///
/// `s.repo.FindOne()`:
/// ```text
/// selector_expression
///   selector_expression
///     identifier "s"
///     field_identifier "repo"
///   field_identifier "FindOne"
/// ```
///
/// Returns `None` for bare `identifier` nodes (single-segment — handled by
/// the existing scope-chain strategies) and for any node we can't walk.
fn build_chain(node: Node, source: &str) -> Option<MemberChain> {
    // Only build a chain for multi-segment expressions.
    if node.kind() == "identifier" {
        return None;
    }
    let mut segments = Vec::new();
    build_chain_inner(node, source, &mut segments)?;
    if segments.len() < 2 {
        return None;
    }
    Some(MemberChain { segments })
}

fn build_chain_inner(node: Node, source: &str, segments: &mut Vec<ChainSegment>) -> Option<()> {
    match node.kind() {
        "identifier" => {
            segments.push(ChainSegment {
                name: node_text(&node, source),
                node_kind: "identifier".to_string(),
                kind: SegmentKind::Identifier,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "selector_expression" => {
            // Children (by index): operand, `.` (anon), field_identifier
            // We need the first named child (operand) and the last named child
            // (field_identifier).  Use indexed access to avoid cursor re-borrow.
            let named_count = node.named_child_count();
            if named_count < 2 {
                return None;
            }
            let operand = node.named_child(0)?;
            let field = node.named_child(named_count - 1)?;

            // Recurse into the operand to build the prefix chain.
            build_chain_inner(operand, source, segments)?;

            segments.push(ChainSegment {
                name: node_text(&field, source),
                node_kind: field.kind().to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "call_expression" => {
            // Nested call in a chain: `a.B().C()` — walk into its function child.
            let func = node.named_child(0)?;
            build_chain_inner(func, source, segments)
        }

        // Unknown node — can't build a chain from this.
        _ => None,
    }
}

/// Emit an `Instantiates` ref for a `composite_literal`.
///
/// `composite_literal` children: type (identifier or qualified_type), literal_value
fn extract_composite_literal_ref(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // The type is the first named child (use index to avoid cursor borrow).
    let type_node = match node.named_child(0) {
        Some(n) => n,
        None => return,
    };

    // Skip if the first named child is the literal_value `{...}` (happens for
    // anonymous composite literals like `{1, 2}`).
    if type_node.kind() == "literal_value" {
        return;
    }

    let type_name = match type_node.kind() {
        "type_identifier" => node_text(&type_node, source),
        "qualified_type" => {
            // `pkg.TypeName` — find the last `type_identifier` by index.
            let last_ti = (0..type_node.named_child_count())
                .filter_map(|i| type_node.named_child(i))
                .filter(|c| c.kind() == "type_identifier")
                .last();
            match last_ti {
                Some(n) => node_text(&n, source),
                None => node_text(&type_node, source),
            }
        }
        _ => node_text(&type_node, source),
    };

    if type_name.is_empty() {
        return;
    }

    refs.push(ExtractedRef {
        source_symbol_index,
        target_name: type_name,
        kind: EdgeKind::Instantiates,
        line: type_node.start_position().row as u32,
        module: None,
        chain: None,
    });
}

// ---------------------------------------------------------------------------
// Signature builders
// ---------------------------------------------------------------------------

/// Build a signature from the first line of the declaration, trimming the
/// opening `{` so it reads as a clean signature.
fn build_fn_signature_from_source(node: &Node, source: &str) -> Option<String> {
    let text = node_text(node, source);
    let first_line = text.lines().next()?;
    let sig = first_line
        .trim_end_matches('{')
        .trim_end()
        .to_string();
    if sig.is_empty() { None } else { Some(sig) }
}

/// Build a signature for a `method_elem` from its source.
///
/// Form: `MethodName(params) result`
fn build_method_elem_signature(node: &Node, source: &str) -> Option<String> {
    let text = node_text(node, source);
    if text.is_empty() { None } else { Some(text) }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn node_text(node: &Node, source: &str) -> String {
    source[node.start_byte()..node.end_byte()].to_string()
}

fn qualify(name: &str, prefix: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}.{name}")
    }
}

fn scope_from_prefix(prefix: &str) -> Option<String> {
    if prefix.is_empty() {
        None
    } else {
        Some(prefix.to_string())
    }
}

/// Go visibility: exported names start with a Unicode uppercase letter.
fn go_visibility(name: &str) -> Option<Visibility> {
    match name.chars().next() {
        Some(c) if c.is_uppercase() => Some(Visibility::Public),
        Some(_) => Some(Visibility::Private),
        None => None,
    }
}

/// Test functions match `TestXxx`, `BenchmarkXxx`, `ExampleXxx`.
fn is_test_function(name: &str) -> bool {
    name.starts_with("Test") || name.starts_with("Benchmark") || name.starts_with("Example")
}

/// Collect consecutive `// ...` line-comment nodes that are unbroken previous
/// siblings of this node and return them as a doc comment string.
fn extract_go_doc_comment(node: &Node, source: &str) -> Option<String> {
    let mut lines: Vec<String> = Vec::new();

    let mut current = node.prev_sibling();
    while let Some(sib) = current {
        match sib.kind() {
            "comment" => {
                let text = node_text(&sib, source);
                if text.starts_with("//") {
                    lines.push(text);
                    current = sib.prev_sibling();
                } else {
                    break;
                }
            }
            _ => break,
        }
    }

    if lines.is_empty() {
        return None;
    }

    lines.reverse();
    Some(lines.join("\n"))
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
fn extract_go_typed_params_as_symbols(
    params_node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    func_qualified_name: &str,
) {
    let mut cursor = params_node.walk();
    for child in params_node.children(&mut cursor) {
        if child.kind() != "parameter_declaration" {
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
            });

            refs.push(ExtractedRef {
                source_symbol_index: param_idx,
                target_name: type_name.clone(),
                kind: EdgeKind::TypeRef,
                line: child.start_position().row as u32,
                module: None,
                chain: None,
            });
        }
    }
}

/// Extract a simple type name from a Go type node for TypeRef emission.
///
/// Handles:
/// - `type_identifier`  → `"Foo"`
/// - `pointer_type`     → `"Foo"` (strips `*`)
/// - `qualified_type`   → `"Foo"` (last segment of `pkg.Foo`)
/// - `slice_type`       → recursively extracts element type
/// - `map_type`         → recursively extracts value type
fn extract_go_type_name(node: &Node, source: &str) -> String {
    match node.kind() {
        "type_identifier" => node_text(node, source),
        "pointer_type" => {
            // Find the inner type.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.is_named() {
                    return extract_go_type_name(&child, source);
                }
            }
            String::new()
        }
        "qualified_type" => {
            // `pkg.Type` — take the last segment.
            let text = node_text(node, source);
            text.rsplit('.').next().unwrap_or(&text).to_string()
        }
        "slice_type" => {
            // `[]Foo` — extract element type.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.is_named() {
                    return extract_go_type_name(&child, source);
                }
            }
            String::new()
        }
        "map_type" => {
            // `map[K]V` — extract value type (second named child).
            let named: Vec<_> = {
                let mut cursor = node.walk();
                node.children(&mut cursor).filter(|c| c.is_named()).collect()
            };
            if named.len() >= 2 {
                return extract_go_type_name(&named[1], source);
            }
            String::new()
        }
        _ => String::new(),
    }
}

/// Return true for Go builtin types that don't reference user symbols.
fn is_go_builtin_type(name: &str) -> bool {
    matches!(
        name,
        "bool" | "byte" | "complex64" | "complex128" | "error"
            | "float32" | "float64"
            | "int" | "int8" | "int16" | "int32" | "int64"
            | "rune" | "string" | "uint" | "uint8" | "uint16"
            | "uint32" | "uint64" | "uintptr"
            | "any" | "comparable"
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "go_tests.rs"]
mod tests;
