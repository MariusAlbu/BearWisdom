use super::helpers::{detect_visibility, node_text};
use super::symbols_casts::{
    extract_type_ref_from_as_expression, extract_type_ref_from_satisfies_expression,
    extract_type_ref_from_type_assertion,
};
use super::types::extract_type_ref_from_annotation;
use crate::parser::scope_tree;
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::Node;

pub(super) fn push_ts_field(
    node: &Node,
    src: &[u8],
    scope_tree: &crate::parser::scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let name = node_text(name_node, src);

    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let qualified_name = scope_tree::qualify(&name, parent_scope);
    let scope_path = scope_tree::scope_path(parent_scope);

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::Property,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: None,
        doc_comment: None,
        scope_path,
        parent_index,
            byte_offset: 0,
    declared_type: None,
    return_type: None,
    param_types: Vec::new(),
    generic_params: Vec::new(),
});

    // Extract TypeRef from field type annotation: `db: DatabaseRepository`
    if let Some(type_ann) = node.child_by_field_name("type") {
        extract_type_ref_from_annotation(&type_ann, src, idx, refs);
    } else if let Some(init) = node.child_by_field_name("value") {
        // No explicit annotation — infer the field's type from its
        // initializer the same way `push_variable_decl` does for locals.
        // Without this, `private _x = new Foo()` leaves `_x`'s type
        // unknown, and downstream chain walks like `this._x.bar()` bail
        // at Phase 2 before reaching the method lookup.
        infer_field_type_from_initializer(init, src, idx, refs);
    }
}

fn infer_field_type_from_initializer(
    init: Node,
    src: &[u8],
    field_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // `private x = await Factory()` — unwrap the await so its inner
    // call_expression drives the inference the same as a plain call.
    let node = if init.kind() == "await_expression" {
        init.child_by_field_name("value")
            .or_else(|| init.named_child(0))
            .unwrap_or(init)
    } else {
        init
    };

    match node.kind() {
        "new_expression" => {
            let Some(constructor) = node.child_by_field_name("constructor") else { return };
            let type_name = match constructor.kind() {
                "identifier" | "type_identifier" => node_text(constructor, src),
                _ => return,
            };
            if type_name.is_empty() {
                return;
            }
            refs.push(ExtractedRef {
                source_symbol_index: field_idx,
                target_name: type_name,
                kind: EdgeKind::TypeRef,
                line: node.start_position().row as u32,
                col: 0,
                module: None,
                chain: None,
                byte_offset: constructor.start_byte() as u32,
                            namespace_segments: Vec::new(),
                            call_args: Vec::new(),
});
            // `new Foo<Bar, Baz>()` — emit each generic arg as an extra
            // TypeRef so the field_type_args map picks them up (mirrors
            // what extract_type_ref_from_annotation does for annotated
            // fields).
            if let Some(type_args) = node.child_by_field_name("type_arguments") {
                let mut tc = type_args.walk();
                for arg in type_args.children(&mut tc) {
                    if arg.kind() == "type_identifier"
                        || arg.kind() == "predefined_type"
                        || arg.kind() == "identifier"
                    {
                        let arg_name = node_text(arg, src);
                        if arg_name.is_empty() {
                            continue;
                        }
                        refs.push(ExtractedRef {
                            source_symbol_index: field_idx,
                            target_name: arg_name,
                            kind: EdgeKind::TypeRef,
                            line: arg.start_position().row as u32,
                            col: 0,
                            module: None,
                            chain: None,
                            byte_offset: arg.start_byte() as u32,
                                                    namespace_segments: Vec::new(),
                                                    call_args: Vec::new(),
});
                    }
                }
            }
        }
        // Intentionally NOT matching `call_expression` / `member_expression`
        // here: their result type is the callee's return type, not the callee
        // itself. Emitting a chain TypeRef pointing at the final method lands
        // on a `method` symbol which fails `kind_compatible(TypeRef, method)`
        // and pollutes unresolved_refs — the Calls ref already emitted by
        // extract.rs's public_field_definition arm carries the same chain
        // and is the right anchor for inheritance-walk resolution anyway.
        "as_expression" => {
            extract_type_ref_from_as_expression(&node, src, field_idx, refs);
        }
        "type_assertion" => {
            extract_type_ref_from_type_assertion(&node, src, field_idx, refs);
        }
        "satisfies_expression" => {
            extract_type_ref_from_satisfies_expression(&node, src, field_idx, refs);
        }
        _ => {}
    }
}
