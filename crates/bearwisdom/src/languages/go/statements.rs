// =============================================================================
// go/statements.rs  —  Statement-form binders: `:=` short-var, `const`, `var`
// =============================================================================

use super::helpers::{
    extract_go_doc_comment, go_visibility, is_go_builtin_type, node_text, pointer_type_name,
    qualify, scope_from_prefix,
};
use super::types::extract_struct_fields;
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::Node;

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
                    byte_offset: 0,
});

        // If the corresponding RHS value is a call_expression, emit a
        // chain-bearing TypeRef so the resolution engine can follow the chain.
        // If it is a composite_literal (e.g. `x := Foo{}`), emit a plain
        // TypeRef for the struct type so the chain resolver can follow it.
        if let Some(rhs_node) = rhs_values.get(i) {
            match rhs_node.kind() {
                "call_expression" => {
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
                                    col: 0,
                                    module: None,
                                    chain: Some(chain),
                                    byte_offset: rhs_node.start_byte() as u32,
                                                                    namespace_segments: Vec::new(),
                                                                    call_args: Vec::new(),
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
                                    col: 0,
                                    module: None,
                                    chain: None,
                                    byte_offset: rhs_node.start_byte() as u32,
                                                                    namespace_segments: Vec::new(),
                                                                    call_args: Vec::new(),
});
                            }
                        }
                    }
                }
                // `x := Foo{}` or `x := pkg.Foo{}` — composite struct literal
                "composite_literal" => {
                    if let Some(type_node) = rhs_node.named_child(0) {
                        if type_node.kind() != "literal_value" {
                            let type_name = match type_node.kind() {
                                "type_identifier" => node_text(&type_node, source),
                                "qualified_type" => {
                                    // `pkg.Foo` — take the last type_identifier
                                    (0..type_node.named_child_count())
                                        .filter_map(|j| type_node.named_child(j))
                                        .filter(|c| c.kind() == "type_identifier")
                                        .last()
                                        .map(|c| node_text(&c, source))
                                        .unwrap_or_default()
                                }
                                _ => node_text(&type_node, source),
                            };
                            if !type_name.is_empty() && !is_go_builtin_type(&type_name) {
                                refs.push(ExtractedRef {
                                    source_symbol_index: sym_idx,
                                    target_name: type_name,
                                    kind: EdgeKind::TypeRef,
                                    line: rhs_node.start_position().row as u32,
                                    col: 0,
                                    module: None,
                                    chain: None,
                                    byte_offset: rhs_node.start_byte() as u32,
                                                                    namespace_segments: Vec::new(),
                                                                    call_args: Vec::new(),
});
                            }
                        }
                    }
                }
                _ => {}
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
/// Extract a clean type name from a var/const spec's type node. Strips
/// pointer prefixes, peels generic instantiation, returns the final
/// identifier for selector expressions, and emits the empty string for
/// anonymous types so the caller skips the TypeRef entirely.
fn extract_var_type_name(node: &Node, source: &str) -> String {
    match node.kind() {
        "type_identifier" => node_text(node, source),
        "pointer_type" => pointer_type_name(node, source),
        "qualified_type" => (0..node.named_child_count())
            .filter_map(|i| node.named_child(i))
            .filter(|c| c.kind() == "type_identifier")
            .last()
            .map(|c| node_text(&c, source))
            .unwrap_or_default(),
        "generic_type" => node
            .named_child(0)
            .map(|n| extract_var_type_name(&n, source))
            .unwrap_or_default(),
        "parenthesized_type" => node
            .named_child(0)
            .map(|n| extract_var_type_name(&n, source))
            .unwrap_or_default(),
        // Anonymous types — no symbolic target to reference.
        "struct_type" | "slice_type" | "map_type" | "array_type"
        | "channel_type" | "function_type" | "interface_type" => String::new(),
        _ => node_text(node, source),
    }
}

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
                type_text = Some(extract_var_type_name(&child, source));
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
                col: 0,
                module: None,
                chain: None,
                byte_offset: node.start_byte() as u32,
                            namespace_segments: Vec::new(),
                            call_args: Vec::new(),
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
                    byte_offset: 0,
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
