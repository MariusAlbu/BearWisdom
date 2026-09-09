//! Field declaration ingestion decodes identifiers before indexing member names.
use super::*;

pub(super) fn extract_positional_fields(
    node: &Node,
    source: &str,
    owner: usize,
    prefix: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let Some(body) = node
        .child_by_field_name("body")
        .filter(|b| b.kind() == super::super::namespaces::FORMS.patterns.ordered_fields)
    else {
        return;
    };
    let mut cursor = body.walk();
    for (index, field) in body
        .named_children(&mut cursor)
        .filter(|n| !n.is_extra() && !matches!(n.kind(), "attribute_item" | "visibility_modifier"))
        .enumerate()
    {
        let name = index.to_string();
        let slot = symbols.len();
        symbols.push(ExtractedSymbol {
            qualified_name: qualify(&name, prefix),
            name,
            kind: SymbolKind::Field,
            visibility: None,
            start_line: field.start_position().row as u32,
            start_col: field.start_position().column as u32,
            end_line: field.end_position().row as u32,
            end_col: field.end_position().column as u32,
            signature: None,
            doc_comment: None,
            scope_path: scope_from_prefix(prefix),
            parent_index: Some(owner),
            byte_offset: field.start_byte() as u32,
            declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
        });
        extract_type_refs_from_type_node(&field, source, slot, refs);
    }
}

pub(in crate::languages::rust_lang) fn extract_struct_fields(
    struct_node: &Node,
    source: &str,
    struct_sym_index: usize,
    qualified_prefix: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let body = match struct_node.child_by_field_name("body") {
        Some(b) => b,
        None => return,
    };

    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() != "field_declaration" {
            continue;
        }

        let name_node = match child.child_by_field_name("name") {
            Some(n) => n,
            None => continue,
        };
        let field_name = node_text(&name_node, source);
        let field_name = field_name
            .strip_prefix(super::super::namespaces::FORMS.raw_prefix)
            .unwrap_or(&field_name)
            .to_owned();
        if field_name.is_empty() {
            continue;
        }

        let qualified_name = qualify(&field_name, qualified_prefix);
        let visibility = detect_visibility(&child);

        // Build a concise signature: `field_name: TypeText`
        let sig = if let Some(type_node) = child.child_by_field_name("type") {
            let type_text = node_text(&type_node, source);
            Some(format!("{field_name}: {type_text}"))
        } else {
            Some(field_name.clone())
        };

        let field_sym_index = symbols.len();
        symbols.push(ExtractedSymbol {
            name: field_name.clone(),
            qualified_name,
            kind: SymbolKind::Field,
            visibility,
            start_line: child.start_position().row as u32,
            end_line: child.end_position().row as u32,
            start_col: child.start_position().column as u32,
            end_col: child.end_position().column as u32,
            signature: sig,
            doc_comment: extract_doc_comment(&child, source),
            scope_path: scope_from_prefix(qualified_prefix),
            parent_index: Some(struct_sym_index),
            byte_offset: 0,
            declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
        });

        // Emit TypeRef for non-primitive field types, attributed to the
        // FIELD's own symbol index (not the struct's) so the field's
        // declared type — not the struct's — is what the ref graph records
        // and what the resolver's TypeRef-derived `field_type_id` sees.
        if let Some(type_node) = child.child_by_field_name("type") {
            extract_type_refs_from_type_node(&type_node, source, field_sym_index, refs);
        }
    }
}

#[cfg(test)]
#[path = "field_declarations_tests.rs"]
mod tests;
