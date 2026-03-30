use super::helpers::node_text;
use crate::types::{EdgeKind, ExtractedRef};
use tree_sitter::Node;

pub(super) fn extract_type_ref_from_annotation(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Walk into type_annotation → the actual type node.
    // type_annotation children: ":" + the type itself
    let type_node = if node.kind() == "type_annotation" {
        let count = node.child_count();
        let mut found = None;
        for i in 0..count {
            if let Some(child) = node.child(i) {
                if child.kind() != ":" {
                    found = Some(child);
                    break;
                }
            }
        }
        found
    } else {
        Some(*node)
    };
    let Some(type_node) = type_node else { return };

    match type_node.kind() {
        "type_identifier" | "identifier" => {
            let type_name = node_text(type_node, src);
            refs.push(ExtractedRef {
                source_symbol_index,
                target_name: type_name,
                kind: EdgeKind::TypeRef,
                line: type_node.start_position().row as u32,
                module: None,
                chain: None,
            });
        }
        "generic_type" => {
            // Repository<User> → extract "Repository" as the ref target,
            // but also emit a second ref with the full generic text for
            // the field_type map to capture type arguments.
            if let Some(name) = type_node.child_by_field_name("name") {
                let base_name = node_text(name, src);
                // Emit base type ref (for edge resolution to the type itself).
                refs.push(ExtractedRef {
                    source_symbol_index,
                    target_name: base_name.clone(),
                    kind: EdgeKind::TypeRef,
                    line: type_node.start_position().row as u32,
                    module: None,
                    chain: None,
                });
                // Also extract type arguments for generic parameter resolution.
                if let Some(type_args_node) = type_node.child_by_field_name("type_arguments") {
                    for i in 0..type_args_node.child_count() {
                        if let Some(arg) = type_args_node.child(i) {
                            if matches!(
                                arg.kind(),
                                "type_identifier" | "identifier" | "generic_type" | "array_type"
                            ) {
                                let arg_name = match arg.kind() {
                                    "generic_type" => arg
                                        .child_by_field_name("name")
                                        .map(|n| node_text(n, src))
                                        .unwrap_or_default(),
                                    "array_type" => {
                                        // User[] → extract "User"
                                        let mut found_name = String::new();
                                        for j in 0..arg.child_count() {
                                            if let Some(child) = arg.child(j) {
                                                if child.kind() == "type_identifier"
                                                    || child.kind() == "identifier"
                                                {
                                                    found_name = node_text(child, src);
                                                    break;
                                                }
                                            }
                                        }
                                        found_name
                                    }
                                    _ => node_text(arg, src),
                                };
                                if !arg_name.is_empty() {
                                    refs.push(ExtractedRef {
                                        source_symbol_index,
                                        target_name: arg_name,
                                        kind: EdgeKind::TypeRef,
                                        line: arg.start_position().row as u32,
                                        module: None,
                                        chain: None,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
        "nested_type_identifier" | "member_expression" => {
            // db.Kysely → extract the full dotted name
            let type_name = node_text(type_node, src);
            refs.push(ExtractedRef {
                source_symbol_index,
                target_name: type_name,
                kind: EdgeKind::TypeRef,
                line: type_node.start_position().row as u32,
                module: None,
                chain: None,
            });
        }
        _ => {}
    }
}

/// Extract TypeRef edges for function/method parameter types and return type.
///
/// For `findAll(id: string, filter: FilterDto): Promise<Album[]>`, emits:
/// - TypeRef from findAll → FilterDto (parameter type)
/// - TypeRef from findAll → Promise (return type)
///
/// Skips primitive types (string, number, boolean, void, any, etc.) since they
/// don't reference user-defined symbols.
pub(super) fn extract_param_and_return_types(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Parameter types.
    if let Some(params) = node.child_by_field_name("parameters") {
        for i in 0..params.child_count() {
            if let Some(param) = params.child(i) {
                if param.kind() == "required_parameter" || param.kind() == "optional_parameter" {
                    if let Some(type_ann) = param.child_by_field_name("type") {
                        extract_type_ref_from_annotation(&type_ann, src, source_symbol_index, refs);
                    }
                }
            }
        }
    }

    // Return type.
    if let Some(ret_type) = node.child_by_field_name("return_type") {
        extract_type_ref_from_annotation(&ret_type, src, source_symbol_index, refs);
    }
}

/// Extract typed function/method parameters as Property symbols scoped to the method.
///
/// For `function getUser(repo: UserRepository)`, creates:
///   Symbol: `getUser.repo` (kind=Property, scope_path=Some("getUser"))
///   TypeRef: `getUser.repo → UserRepository`
///
/// This enables chain resolution inside the function body:
/// `repo.findOne()` resolves because `getUser.repo` is in `field_type` as `UserRepository`.
pub(super) fn extract_typed_params_as_symbols(
    func_node: &Node,
    src: &[u8],
    scope_tree: &crate::parser::scope_tree::ScopeTree,
    symbols: &mut Vec<crate::types::ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    use crate::parser::scope_tree;
    use crate::types::SymbolKind;

    let params = match func_node.child_by_field_name("parameters") {
        Some(p) => p,
        None => return,
    };

    // The method scope — parameters should be qualified under the method name.
    // find_scope_at at the param position should give us the method scope
    // (since method_definition is now in TS_SCOPE_KINDS).
    let method_scope = if func_node.start_byte() > 0 {
        // Use the byte inside the parameters to find the method scope.
        scope_tree::find_scope_at(scope_tree, params.start_byte())
    } else {
        None
    };

    for i in 0..params.child_count() {
        let Some(param) = params.child(i) else { continue };
        if param.kind() != "required_parameter" && param.kind() != "optional_parameter" {
            continue;
        }

        // Skip constructor parameter properties (handled by extract_constructor_params).
        let has_modifier = (0..param.child_count()).any(|j| {
            param
                .child(j)
                .map(|c| c.kind() == "accessibility_modifier" || c.kind() == "readonly")
                .unwrap_or(false)
        });
        if has_modifier {
            continue;
        }

        // Get the parameter name.
        let name_node = match param
            .child_by_field_name("pattern")
            .or_else(|| param.child_by_field_name("name"))
        {
            Some(n) if n.kind() == "identifier" => n,
            _ => continue,
        };

        // Must have a type annotation — skip untyped parameters.
        let type_ann = match param.child_by_field_name("type") {
            Some(t) => t,
            None => continue,
        };

        let name = node_text(name_node, src);
        let qualified_name = scope_tree::qualify(&name, method_scope);
        let scope_path = scope_tree::scope_path(method_scope);

        let idx = symbols.len();
        symbols.push(crate::types::ExtractedSymbol {
            name,
            qualified_name,
            kind: SymbolKind::Property,
            visibility: None,
            start_line: param.start_position().row as u32,
            end_line: param.end_position().row as u32,
            start_col: param.start_position().column as u32,
            end_col: param.end_position().column as u32,
            signature: None,
            doc_comment: None,
            scope_path,
            parent_index,
        });

        // Emit TypeRef from the parameter symbol to its type.
        extract_type_ref_from_annotation(&type_ann, src, idx, refs);
    }
}
