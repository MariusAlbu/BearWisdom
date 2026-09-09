use super::*;

/// Also emits TypeRef edges for any named types in variant field declarations.
pub(in crate::languages::rust_lang) fn extract_enum_variants(
    body: &Node,
    source: &str,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() == "enum_variant" {
            // tree-sitter-rust uses `name` field on enum_variant nodes.
            // Fall back to the first named identifier child if the field is missing.
            let field_name_node = child.child_by_field_name("name");
            let name_node = if field_name_node.is_some() {
                field_name_node
            } else {
                let mut variant_cursor = child.walk();
                let found = child
                    .children(&mut variant_cursor)
                    .find(|n| n.is_named() && n.kind() == "identifier");
                found
            };

            if let Some(name_node) = name_node {
                let name = node_text(&name_node, source);
                let name = name
                    .strip_prefix(super::super::namespaces::FORMS.raw_prefix)
                    .unwrap_or(&name)
                    .to_owned();
                let qualified_name = qualify(&name, qualified_prefix);
                let sym_idx = symbols.len();
                symbols.push(ExtractedSymbol {
                    name,
                    qualified_name,
                    kind: SymbolKind::EnumMember,
                    visibility: None,
                    start_line: child.start_position().row as u32,
                    end_line: child.end_position().row as u32,
                    start_col: child.start_position().column as u32,
                    end_col: child.end_position().column as u32,
                    signature: None,
                    doc_comment: extract_doc_comment(&child, source),
                    scope_path: scope_from_prefix(qualified_prefix),
                    parent_index,
                    byte_offset: 0,
                    declared_type: None,
                    return_type: None,
                    param_types: Vec::new(),
                    generic_params: Vec::new(),
                });

                let prefix = symbols[sym_idx].qualified_name.clone();
                super::fields::extract_struct_fields(
                    &child, source, sym_idx, &prefix, symbols, refs,
                );
                super::fields::extract_positional_fields(
                    &child, source, sym_idx, &prefix, symbols, refs,
                );

                // Extract attributes on the enum variant (e.g. #[default], #[serde(rename="...")]).
                super::super::decorators::extract_decorators(&child, source, sym_idx, refs);

                // Emit TypeRefs for any typed fields in the variant body.
                // Covers tuple variants `Error(ErrorKind)` and struct variants
                // `Point { x: f32, y: f32 }` whose field types are type_identifiers.
                let mut vc = child.walk();
                for variant_child in child.children(&mut vc) {
                    match variant_child.kind() {
                        // Tuple variant: `Error(ErrorKind, String)`
                        "ordered_field_declaration_list" => {
                            let mut fc = variant_child.walk();
                            for field in variant_child.children(&mut fc) {
                                if field.kind() == "type" || field.is_named() {
                                    extract_type_refs_from_type_node(&field, source, sym_idx, refs);
                                }
                            }
                        }
                        // Struct variant: `Point { x: f32, y: f32 }`
                        "field_declaration_list" => {
                            let mut fc = variant_child.walk();
                            for field in variant_child.children(&mut fc) {
                                if field.kind() == "field_declaration" {
                                    if let Some(type_node) = field.child_by_field_name("type") {
                                        extract_type_refs_from_type_node(
                                            &type_node, source, sym_idx, refs,
                                        );
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "variant_declarations_tests.rs"]
mod tests;
