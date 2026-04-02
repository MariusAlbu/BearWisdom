use super::helpers::node_text;
use crate::types::{EdgeKind, ExtractedRef};
use tree_sitter::Node;

pub(super) fn push_import(
    node: &Node,
    src: &[u8],
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Import source: `import { X, Y } from './foo'` or `import Foo from 'bar'`
    let module_path = node
        .child_by_field_name("source")
        .map(|s| {
            node_text(s, src)
                .trim_matches('"')
                .trim_matches('\'')
                .to_string()
        });

    // Named imports: `{ X, Y as Z }` → push one ref per binding.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_clause" => {
                let mut ic = child.walk();
                for item in child.children(&mut ic) {
                    match item.kind() {
                        "identifier" => {
                            // Default import: `import Foo from ...`
                            refs.push(ExtractedRef {
                                source_symbol_index: current_symbol_count,
                                target_name: node_text(item, src),
                                kind: EdgeKind::TypeRef,
                                line: item.start_position().row as u32,
                                module: module_path.clone(),
                                chain: None,
                            });
                        }
                        "named_imports" => {
                            let mut ni = item.walk();
                            for spec in item.children(&mut ni) {
                                if spec.kind() == "import_specifier" {
                                    // `name` = imported name, `alias` = local alias.
                                    // We use the imported name for resolution.
                                    let imported_name = spec
                                        .child_by_field_name("name")
                                        .map(|n| node_text(n, src))
                                        .unwrap_or_else(|| node_text(spec, src));
                                    refs.push(ExtractedRef {
                                        source_symbol_index: current_symbol_count,
                                        target_name: imported_name,
                                        kind: EdgeKind::TypeRef,
                                        line: spec.start_position().row as u32,
                                        module: module_path.clone(),
                                        chain: None,
                                    });
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
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
                        });
                    }
                }
            }
            _ => {}
        }
    }
}
