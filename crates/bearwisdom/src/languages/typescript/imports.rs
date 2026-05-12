use super::helpers::node_text;
use crate::ecosystem::ecmascript_imports::{push_import_refs, PushImportOpts};
use crate::types::{EdgeKind, ExtractedRef};
use tree_sitter::Node;

pub(super) fn push_import(
    node: &Node,
    src: &[u8],
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    push_import_refs(node, src, current_symbol_count, refs, PushImportOpts::TYPESCRIPT);
}

// ---------------------------------------------------------------------------
// Heritage clause (extends / implements)
// ---------------------------------------------------------------------------

pub(super) fn extract_heritage(
    node: &Node,
    src: &[u8],
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_heritage" => {
                let mut hc = child.walk();
                for clause in child.children(&mut hc) {
                    match clause.kind() {
                        "extends_clause" => {
                            let mut ec = clause.walk();
                            for type_node in clause.children(&mut ec) {
                                if type_node.kind() == "identifier"
                                    || type_node.kind() == "type_identifier"
                                {
                                    refs.push(ExtractedRef {
                                        source_symbol_index: source_idx,
                                        target_name: node_text(type_node, src),
                                        kind: EdgeKind::Inherits,
                                        line: type_node.start_position().row as u32,
                                        module: None,
                                        chain: None,
                                        byte_offset: 0,
                                                                            namespace_segments: Vec::new(),
                                                                            call_args: Vec::new(),
});
                                }
                            }
                        }
                        "implements_clause" => {
                            let mut ic = clause.walk();
                            for type_node in clause.children(&mut ic) {
                                if type_node.kind() == "type_identifier"
                                    || type_node.kind() == "identifier"
                                {
                                    refs.push(ExtractedRef {
                                        source_symbol_index: source_idx,
                                        target_name: node_text(type_node, src),
                                        kind: EdgeKind::Implements,
                                        line: type_node.start_position().row as u32,
                                        module: None,
                                        chain: None,
                                        byte_offset: 0,
                                                                            namespace_segments: Vec::new(),
                                                                            call_args: Vec::new(),
});
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            "extends_clause" => {
                // Direct child for interfaces.
                let mut ec = child.walk();
                for type_node in child.children(&mut ec) {
                    if type_node.kind() == "identifier" || type_node.kind() == "type_identifier" {
                        refs.push(ExtractedRef {
                            source_symbol_index: source_idx,
                            target_name: node_text(type_node, src),
                            kind: EdgeKind::Inherits,
                            line: type_node.start_position().row as u32,
                            module: None,
                            chain: None,
                            byte_offset: 0,
                                                    namespace_segments: Vec::new(),
                                                    call_args: Vec::new(),
});
                    }
                }
            }
            // `extends_type_clause` is the TS grammar node for interface inheritance:
            // `interface B extends A, C` -- distinct from `extends_clause` used for classes.
            "extends_type_clause" => {
                let mut ec = child.walk();
                for type_node in child.children(&mut ec) {
                    if type_node.kind() == "type_identifier" || type_node.kind() == "identifier" {
                        refs.push(ExtractedRef {
                            source_symbol_index: source_idx,
                            target_name: node_text(type_node, src),
                            kind: EdgeKind::Inherits,
                            line: type_node.start_position().row as u32,
                            module: None,
                            chain: None,
                            byte_offset: 0,
                                                    namespace_segments: Vec::new(),
                                                    call_args: Vec::new(),
});
                    }
                }
            }
            _ => {}
        }
    }
}
