// =============================================================================
// go/symbols.rs  —  Symbol extraction for Go declarations
// =============================================================================

use super::calls::extract_body_with_symbols;
use super::helpers::{
    build_fn_signature_from_source, build_method_elem_signature, extract_go_doc_comment,
    extract_go_type_name, go_visibility, is_go_builtin_type, is_test_function, node_text,
    pointer_type_name, qualify, scope_from_prefix,
};
use super::tags;
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
// Type declarations
// ---------------------------------------------------------------------------

pub(super) fn extract_type_declaration(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "type_spec" => {
                extract_type_spec(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                );
            }
            // `type Foo = Bar` — tree-sitter-go 0.23+ uses a distinct `type_alias` node.
            // Fields: `name` (type_identifier), `type` (_type)
            "type_alias" => {
                extract_type_alias_decl(
                    &child,
                    source,
                    symbols,
                    parent_index,
                    qualified_prefix,
                );
            }
            _ => {}
        }
    }
}

/// Extract a `type_alias` node (`type Foo = Bar`).
///
/// tree-sitter-go shape:
/// ```text
/// type_alias
///   name: type_identifier   "Foo"
///   "="                     (anonymous)
///   type: _type             "Bar"
/// ```
fn extract_type_alias_decl(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let name = node_text(&name_node, source);
    if name.is_empty() {
        return;
    }

    let type_text = node
        .child_by_field_name("type")
        .map(|n| node_text(&n, source))
        .unwrap_or_default();

    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = go_visibility(&name);
    let doc_comment = extract_go_doc_comment(node, source);
    let sig = if type_text.is_empty() {
        format!("type {name} =")
    } else {
        format!("type {name} = {type_text}")
    };

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
    //
    // For generic types like `type Result[T any] struct { ... }`, tree-sitter-go
    // emits a `type_parameter_list` or `type_parameter_declaration` node BEFORE
    // the actual type body.  Skip over those so we find the struct_type / interface_type.
    let type_node = match named_children
        .into_iter()
        .find(|n| !matches!(n.kind(), "type_parameter_list" | "type_parameter_declaration" | "type_constraints"))
    {
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
            extract_interface_methods_with_refs(&type_node, source, symbols, refs, Some(idx), &iface_prefix);
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
    let mut tag_doc: Option<String> = None;

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
            // `pkg.Type` embedded field (e.g. `http.Handler`, `sync.Mutex`).
            "qualified_type" if field_names.is_empty() && type_text.is_none() => {
                let leaf = (0..child.named_child_count())
                    .filter_map(|i| child.named_child(i))
                    .filter(|c| c.kind() == "type_identifier")
                    .last();
                embedded_type = Some(match leaf {
                    Some(n) => node_text(&n, source),
                    None => node_text(&child, source),
                });
            }
            "raw_string_literal" => {
                // Struct tag: `json:"name" db:"col"`
                let raw = node_text(&child, source);
                let parsed = tags::parse_struct_tags(&raw);
                if !parsed.is_empty() {
                    tag_doc = Some(tags::format_tags(&parsed));
                }
            }
            _ => {
                // Any other named child after field_identifier(s) is the type.
                if !field_names.is_empty() {
                    type_text = Some(node_text(&child, source));
                    // Walk the type subtree to emit TypeRef for every
                    // type_identifier within it (handles slices, maps, pointers,
                    // channels, and nested anonymous structs).
                    emit_type_refs_from_subtree(
                        &child,
                        source,
                        parent_index.unwrap_or(0),
                        refs,
                        struct_prefix,
                        symbols,
                        parent_index,
                    );
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

        // Emit a TypeRef for the field's type when it is a user-defined
        // named type (i.e., a type_identifier that is not a Go builtin).
        // We do this once per field_declaration regardless of how many
        // field names are listed (they all share the same type).
        if !type_str.is_empty() && !is_go_builtin_type(&type_str) {
            refs.push(ExtractedRef {
                source_symbol_index: parent_index.unwrap_or(0),
                target_name: type_str.clone(),
                kind: EdgeKind::TypeRef,
                line: node.start_position().row as u32,
                module: None,
                chain: None,
            });
        }

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
                doc_comment: tag_doc.clone(),
                scope_path: scope_from_prefix(struct_prefix),
                parent_index,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Type subtree walker (TypeRef emission + nested struct field recursion)
// ---------------------------------------------------------------------------

/// Walk a Go type node and:
///   1. Emit `TypeRef` for every `type_identifier` that is not a builtin.
///   2. Recurse into `struct_type` → `field_declaration_list` so that nested
///      anonymous struct fields are also captured as `Field` symbols.
///
/// This ensures that complex field types like `[]*Handler`, `map[string]User`,
/// `chan Event`, and inline `struct { … }` all produce the correct coverage.
fn emit_type_refs_from_subtree(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
    struct_prefix: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    match node.kind() {
        "type_identifier" => {
            let name = node_text(node, source);
            if !name.is_empty() && !is_go_builtin_type(&name) {
                refs.push(ExtractedRef {
                    source_symbol_index,
                    target_name: name,
                    kind: EdgeKind::TypeRef,
                    line: node.start_position().row as u32,
                    module: None,
                    chain: None,
                });
            }
        }

        // Nested anonymous struct — recurse into its field_declaration_list
        // so the inner field_declaration nodes are also captured.
        "struct_type" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
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

        // For all other container types (slice_type, map_type, pointer_type,
        // channel_type, array_type, qualified_type, etc.) just recurse into
        // named children to find any type_identifier nodes within.
        _ => {
            if node.is_named() && node.child_count() > 0 {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.is_named() {
                        emit_type_refs_from_subtree(
                            &child,
                            source,
                            source_symbol_index,
                            refs,
                            struct_prefix,
                            symbols,
                            parent_index,
                        );
                    }
                }
            }
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
    // Use a local refs buffer; we don't need a `refs` parameter since
    // callers don't thread it through, but we emit TypeRefs via calls.
    // Actually, we need refs from the outer context. Add a dummy approach:
    // defer to the fn_signature extractor for param/return type TypeRefs.
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

fn extract_interface_methods_with_refs(
    iface_node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    iface_prefix: &str,
) {
    let mut cursor = iface_node.walk();
    for child in iface_node.children(&mut cursor) {
        if child.kind() != "method_elem" {
            continue;
        }

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
        let sym_idx = symbols.len();

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

        // Emit TypeRef edges for parameter and return types of this method_elem.
        super::calls::extract_fn_signature_type_refs(&child, source, sym_idx, refs);
    }
}

// ---------------------------------------------------------------------------
// Short variable declarations (`:=`)
// ---------------------------------------------------------------------------

/// Extract a `short_var_declaration` node.
///
/// `user := repo.FindOne(id)` or `data, err := fetchData()`
///
/// Tree-sitter-go shape:
/// ```text
/// short_var_declaration
///   expression_list      ← left  (identifiers)
///   ":="                 (anon)
///   expression_list      ← right (values / call expressions)
/// ```
///
/// For each declared name emit a Variable symbol.  When the corresponding
/// right-hand value is a `call_expression`, emit a chain-bearing TypeRef so the
/// resolution engine can infer the variable's type from the callee's return type.
pub(super) fn extract_short_var_decl(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    enclosing_symbol_index: usize,
) {
    use super::calls::build_chain;

    // Collect named children — first expression_list is LHS, second is RHS.
    let named: Vec<Node> = {
        let mut cursor = node.walk();
        node.children(&mut cursor)
            .filter(|c| c.is_named())
            .collect()
    };

    if named.len() < 2 {
        return;
    }

    let lhs = &named[0];
    let rhs = &named[1];

    // Collect the LHS identifiers.
    let lhs_names: Vec<(String, u32, u32)> = {
        let mut cursor = lhs.walk();
        lhs.children(&mut cursor)
            .filter(|c| c.is_named() && c.kind() == "identifier")
            .map(|c| {
                (
                    node_text(&c, source),
                    c.start_position().row as u32,
                    c.start_position().column as u32,
                )
            })
            .collect()
    };

    if lhs_names.is_empty() {
        return;
    }

    // Collect the RHS values (call expressions or other).
    let rhs_values: Vec<Node> = {
        let mut cursor = rhs.walk();
        rhs.children(&mut cursor).filter(|c| c.is_named()).collect()
    };

    for (i, (name, start_line, start_col)) in lhs_names.iter().enumerate() {
        // Skip blank identifiers.
        if name == "_" {
            continue;
        }

        let qualified_name = qualify(name, qualified_prefix);
        let visibility = go_visibility(name);

        let sym_idx = symbols.len();
        symbols.push(ExtractedSymbol {
            name: name.clone(),
            qualified_name,
            kind: SymbolKind::Variable,
            visibility,
            start_line: *start_line,
            end_line: node.end_position().row as u32,
            start_col: *start_col,
            end_col: node.end_position().column as u32,
            signature: Some(format!("{name} :=")),
            doc_comment: None,
            scope_path: scope_from_prefix(qualified_prefix),
            parent_index,
        });

        // If the corresponding RHS value is a call_expression, emit a
        // chain-bearing TypeRef so the resolution engine can follow the chain.
        if let Some(rhs_node) = rhs_values.get(i) {
            if rhs_node.kind() == "call_expression" {
                if let Some(func) = rhs_node.named_child(0) {
                    if let Some(chain) = build_chain(func, source) {
                        let target = chain
                            .segments
                            .last()
                            .map(|s| s.name.clone())
                            .unwrap_or_default();
                        if !target.is_empty() {
                            refs.push(ExtractedRef {
                                source_symbol_index: sym_idx,
                                target_name: target,
                                kind: EdgeKind::TypeRef,
                                line: rhs_node.start_position().row as u32,
                                module: None,
                                chain: Some(chain),
                            });
                        }
                    } else {
                        // Bare function call (single identifier) — still emit TypeRef.
                        let target = node_text(&func, source);
                        if !target.is_empty() && target != "_" {
                            refs.push(ExtractedRef {
                                source_symbol_index: sym_idx,
                                target_name: target,
                                kind: EdgeKind::TypeRef,
                                line: rhs_node.start_position().row as u32,
                                module: None,
                                chain: None,
                            });
                        }
                    }
                }
            }
        }

        // Recurse into the RHS call expressions to extract any nested calls.
        // We do this via the body extractor on the full RHS node.
        if let Some(rhs_node) = rhs_values.get(i) {
            super::calls::extract_refs_from_body(rhs_node, source, enclosing_symbol_index, refs);
        }
    }
}

// ---------------------------------------------------------------------------
// Const / var declarations
// ---------------------------------------------------------------------------

pub(super) fn extract_const_var_decl(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    keyword: &str,
    spec_kind: &str,
) {
    // Derive the list-wrapper kind: "var_spec" → "var_spec_list",
    // "const_spec" → "const_spec_list".  Grouped declarations
    // (`var ( ... )` / `const ( ... )`) use an intermediate list node.
    let list_kind = match spec_kind {
        "var_spec" => "var_spec_list",
        "const_spec" => "const_spec_list",
        other => {
            // Fallback: append "_list" and hope for the best.
            let fallback = format!("{other}_list");
            return extract_const_var_decl_inner(
                node, source, symbols, refs, parent_index, qualified_prefix, keyword, spec_kind, &fallback,
            );
        }
    };
    extract_const_var_decl_inner(
        node, source, symbols, refs, parent_index, qualified_prefix, keyword, spec_kind, list_kind,
    );
}

fn extract_const_var_decl_inner(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    keyword: &str,
    spec_kind: &str,
    list_kind: &str,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == spec_kind {
            extract_const_var_spec(
                &child,
                source,
                symbols,
                refs,
                parent_index,
                qualified_prefix,
                keyword,
            );
        } else if child.kind() == list_kind {
            // Grouped `var ( ... )` / `const ( ... )` wraps specs in a list node.
            let mut lc = child.walk();
            for spec in child.children(&mut lc) {
                if spec.kind() == spec_kind {
                    extract_const_var_spec(
                        &spec,
                        source,
                        symbols,
                        refs,
                        parent_index,
                        qualified_prefix,
                        keyword,
                    );
                }
            }
        }
    }
}

/// `const_spec` / `var_spec` children:
///   identifier+ (names), [type], [= expression_list]
fn extract_const_var_spec(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    keyword: &str,
) {
    let mut names: Vec<(String, u32, u32)> = Vec::new();
    let mut type_text: Option<String> = None;
    let mut type_node_for_struct: Option<Node> = None;
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
                // When the declared type is an anonymous struct (e.g.
                // `var opts struct { Verbose bool }`), remember the node so we
                // can extract its field_declaration children below.
                if child.kind() == "struct_type" {
                    type_node_for_struct = Some(child);
                }
                type_text = Some(node_text(&child, source));
                past_names = true;
            }
            _ => {}
        }
    }

    // Emit TypeRef for a declared type that references a user-defined symbol.
    // We do this once, not per-name, because all names share the same type.
    if let Some(ref t) = type_text {
        if !t.is_empty() && !is_go_builtin_type(t) {
            refs.push(ExtractedRef {
                source_symbol_index: parent_index.unwrap_or(0),
                target_name: t.clone(),
                kind: EdgeKind::TypeRef,
                line: node.start_position().row as u32,
                module: None,
                chain: None,
            });
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

        let var_idx = symbols.len();
        symbols.push(ExtractedSymbol {
            name: name.clone(),
            qualified_name: qualified_name.clone(),
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

        // When the declared type is an anonymous struct, extract its fields as
        // Field symbols scoped to the variable (e.g. `var opts struct{ Verbose bool }`
        // → Field symbols `opts.Verbose`).
        if let Some(ref struct_node) = type_node_for_struct {
            extract_struct_fields(
                struct_node,
                source,
                symbols,
                refs,
                Some(var_idx),
                &qualified_name,
            );
        }
    }
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

// ---------------------------------------------------------------------------
// Inline struct field extraction
// ---------------------------------------------------------------------------

/// Walk an arbitrary expression subtree looking for anonymous `struct_type`
/// nodes and extract their fields.  This covers patterns like:
///
///   `data := struct{ Name string }{...}`
///   `rows := []struct{ URL string; Status int }{{...}, {...}}`
///
/// We stop descending into `function_literal` / `func_literal` nodes so we
/// don't accidentally steal fields from closures declared inside the RHS.
pub(super) fn extract_inline_struct_fields(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    match node.kind() {
        "struct_type" => {
            extract_struct_fields(node, source, symbols, refs, parent_index, qualified_prefix);
        }
        // Don't descend into closures — they are separate symbols.
        "function_literal" | "func_literal" => {}
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.is_named() {
                    extract_inline_struct_fields(
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
    }
}
