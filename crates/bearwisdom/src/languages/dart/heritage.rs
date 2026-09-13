// =============================================================================
// dart/heritage.rs  —  supertype clauses of a Dart class, mixin or enum
// =============================================================================

use super::helpers::node_text;
use crate::types::{EdgeKind, ExtractedRef};
use tree_sitter::Node;

pub(super) fn extract_dart_heritage(
    node: &Node,
    src: &str,
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // `superclass`, `interfaces`, and `mixins` are named FIELDS on class_definition /
    // mixin_declaration, NOT plain children.  node.children() skips named fields —
    // they must be accessed via child_by_field_name().

    // `: extends Foo` → Inherits
    // Grammar: superclass node has a `type` field containing type_identifier.
    if let Some(superclass_node) = node.child_by_field_name("superclass") {
        if let Some(type_node) = superclass_node.child_by_field_name("type") {
            let name = node_text(type_node, src);
            if !name.is_empty() {
                refs.push(ExtractedRef {
                    is_include: false,
                    is_import_binding: false,
                    is_reexport: false,
                    source_symbol_index: source_idx,
                    target_name: name,
                    kind: EdgeKind::Inherits,
                    line: type_node.start_position().row as u32,
                    col: 0,
                    module: None,
                    chain: None,
                    byte_offset: type_node.start_byte() as u32,
                    namespace_segments: Vec::new(),
                    call_args: Vec::new(),
                });
            }
        } else {
            // Fallback: scan children of superclass for type_identifier.
            let mut c = superclass_node.walk();
            for n in superclass_node.children(&mut c) {
                if n.kind() == "type_identifier" || n.kind() == "identifier" {
                    refs.push(ExtractedRef {
                        is_include: false,
                        is_import_binding: false,
                        is_reexport: false,
                        source_symbol_index: source_idx,
                        target_name: node_text(n, src),
                        kind: EdgeKind::Inherits,
                        line: n.start_position().row as u32,
                        col: 0,
                        module: None,
                        chain: None,
                        byte_offset: n.start_byte() as u32,
                        namespace_segments: Vec::new(),
                        call_args: Vec::new(),
                    });
                }
            }
        }
    }

    // `implements Foo, Bar` → Implements (one edge per interface)
    // Grammar: interfaces node has children() of type_identifier (no named fields).
    if let Some(interfaces_node) = node.child_by_field_name("interfaces") {
        let mut c = interfaces_node.walk();
        for n in interfaces_node.children(&mut c) {
            if n.kind() == "type_identifier" || n.kind() == "identifier" {
                refs.push(ExtractedRef {
                    is_include: false,
                    is_import_binding: false,
                    is_reexport: false,
                    source_symbol_index: source_idx,
                    target_name: node_text(n, src),
                    kind: EdgeKind::Implements,
                    line: n.start_position().row as u32,
                    col: 0,
                    module: None,
                    chain: None,
                    byte_offset: n.start_byte() as u32,
                    namespace_segments: Vec::new(),
                    call_args: Vec::new(),
                });
            }
        }
    }

    // `with Mixin1, Mixin2` → TypeRef (mixins are applied, not inherited)
    // The mixins field may live directly on the class node or as a child of the
    // superclass node (when `class C extends Base with Mixin {}`).
    let mut emit_mixin_refs = |mixins_node: Node| {
        let mut c = mixins_node.walk();
        for n in mixins_node.children(&mut c) {
            if n.kind() == "type_identifier" || n.kind() == "identifier" {
                refs.push(ExtractedRef {
                    is_include: false,
                    is_import_binding: false,
                    is_reexport: false,
                    source_symbol_index: source_idx,
                    target_name: node_text(n, src),
                    kind: EdgeKind::TypeRef,
                    line: n.start_position().row as u32,
                    col: 0,
                    module: None,
                    chain: None,
                    byte_offset: n.start_byte() as u32,
                    namespace_segments: Vec::new(),
                    call_args: Vec::new(),
                });
            }
        }
    };

    if let Some(mixins_node) = node.child_by_field_name("mixins") {
        emit_mixin_refs(mixins_node);
    }
    if let Some(sc) = node.child_by_field_name("superclass") {
        if let Some(mixins_node) = sc.child_by_field_name("mixins") {
            emit_mixin_refs(mixins_node);
        } else {
            // Fallback: scan sc children for a `mixins` node.
            // Collect first to avoid TreeCursor lifetime issues.
            let mut c = sc.walk();
            let sc_children: Vec<Node> = sc.children(&mut c).collect();
            if let Some(mixins_node) = sc_children.iter().find(|n| n.kind() == "mixins") {
                emit_mixin_refs(*mixins_node);
            }
        }
    }
}
