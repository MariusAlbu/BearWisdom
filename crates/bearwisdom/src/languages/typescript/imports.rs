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
                                byte_offset: 0,
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
                                        byte_offset: 0,
                                    });
                                }
                            }
                        }
                        // `import * as ns from './bar'` -- namespace import.
                        // The local alias `ns` is the identifier child of namespace_import.
                        "namespace_import" => {
                            let mut ni = item.walk();
                            for ns_child in item.children(&mut ni) {
                                if ns_child.kind() == "identifier" {
                                    refs.push(ExtractedRef {
                                        source_symbol_index: current_symbol_count,
                                        target_name: node_text(ns_child, src),
                                        kind: EdgeKind::TypeRef,
                                        line: ns_child.start_position().row as u32,
                                        module: module_path.clone(),
                                        chain: None,
                                        byte_offset: 0,
                                    });
                                    break;
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            // `import path = require("mod")` -- CommonJS-style TypeScript import.
            // import_require_clause children: identifier (local name), `=`, `require`,
            // `(`, string (module path), `)`.
            "import_require_clause" => {
                // Walk the clause children once, collecting both the local name and
                // the module string. Explicit loops avoid borrow conflicts from
                // iterator adapters that hold a reference to the walker.
                let mut local_name = String::new();
                let mut require_module: Option<String> = None;
                let mut rc = child.walk();
                for rc_child in child.children(&mut rc) {
                    match rc_child.kind() {
                        "identifier" if local_name.is_empty() => {
                            local_name = node_text(rc_child, src);
                        }
                        "string" => {
                            let raw = node_text(rc_child, src);
                            require_module =
                                Some(raw.trim_matches('"').trim_matches('\'').to_string());
                        }
                        _ => {}
                    }
                }
                if !local_name.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index: current_symbol_count,
                        target_name: local_name,
                        kind: EdgeKind::Imports,
                        line: child.start_position().row as u32,
                        module: require_module,
                        chain: None,
                        byte_offset: 0,
                    });
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
                                        byte_offset: 0,
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
                        });
                    }
                }
            }
            _ => {}
        }
    }
}
