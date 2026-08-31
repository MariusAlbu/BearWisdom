// =============================================================================
// go/param_symbols.rs  —  Parameter-list type extraction
//
// Turns a Go `parameter_list` into derived data: the receiver's bare type
// name (for method qualification) and typed parameters as Property symbols
// scoped to the enclosing function or method. Distinct from symbols.rs, which
// owns declaration-level symbol framing (package/import/function/method) and
// delegates parameter handling here.
// =============================================================================

use super::helpers::{is_go_builtin_type, node_text, pointer_type_name, qualify};
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::Node;

/// Extract the plain type name from a receiver `parameter_list`.
///
/// `(p Point)` or `(s *Server)` → `"Point"` / `"Server"`.
pub(super) fn extract_receiver_type_from_param_list(
    param_list: &Node,
    source: &str,
) -> Option<String> {
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
        let type_node = match type_node {
            Some(tn) => tn,
            None => continue,
        };

        // The signature renders the type's raw source text (`*testing.T`,
        // faithful to what was written); the TypeRef resolves against the
        // bare name plus its package qualifier.
        let sig_type_text = node_text(&type_node, source);
        let (target_name, module) =
            match super::qualified_types::go_type_ref_target(&type_node, source) {
                Some((name, _)) if name.is_empty() || is_go_builtin_type(&name) => continue,
                Some(parts) => parts,
                None => continue,
            };

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
                signature: Some(format!("{name} {sig_type_text}")),
                doc_comment: None,
                scope_path,
                parent_index,
                byte_offset: 0,
                declared_type: None,
                return_type: None,
                param_types: Vec::new(),
                generic_params: Vec::new(),
            });

            refs.push(ExtractedRef {
                is_include: false,
                is_import_binding: false,
                is_reexport: false,
                source_symbol_index: param_idx,
                target_name: target_name.clone(),
                kind: EdgeKind::TypeRef,
                line: child.start_position().row as u32,
                col: 0,
                module: module.clone(),
                chain: None,
                byte_offset: child.start_byte() as u32,
                namespace_segments: Vec::new(),
                call_args: Vec::new(),
            });
        }
    }
}

#[cfg(test)]
#[path = "param_symbols_tests.rs"]
mod tests;
