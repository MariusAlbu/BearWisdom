// =============================================================================
// languages/groovy/ast_visit.rs  —  AST traversal and per-construct extractors
//
// Walks the tree-sitter parse tree dispatching to one extractor per top-level
// construct: package, class, interface, function, method, import, call.
// =============================================================================

use crate::types::{
    EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility,
};
use super::predicates;
use super::calls::{build_receiver_chain, scan_local_types, visit_for_calls};
use super::node_helpers::{build_qualified_name, named_field_text, node_text};
use std::collections::HashMap;
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// Core traversal
// ---------------------------------------------------------------------------

pub(super) fn visit(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    inside_class: bool,
    namespace: Option<&str>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "package_declaration" => {
                extract_package(&child, src, symbols, parent_index);
            }
            "class_declaration" => {
                extract_class(&child, src, symbols, refs, parent_index, namespace);
            }
            "interface_declaration" => {
                extract_interface(&child, src, symbols, refs, parent_index, namespace);
            }
            // Top-level `def fn(...)` — grammar emits function_definition
            "function_definition" => {
                extract_function(&child, src, symbols, refs, parent_index, inside_class, None);
            }
            // Typed `ReturnType method(...)` inside a class — grammar emits method_declaration
            "method_declaration" => {
                extract_method_declaration(&child, src, symbols, refs, parent_index, None);
            }
            "import_declaration" => {
                extract_import(&child, src, symbols.len().saturating_sub(1), refs);
            }
            "method_invocation" => {
                let local_types = HashMap::new();
                extract_call(&child, src, parent_index.unwrap_or(0), refs, &local_types);
                visit(child, src, symbols, refs, parent_index, inside_class, namespace);
            }
            _ => {
                visit(child, src, symbols, refs, parent_index, inside_class, namespace);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Package / Namespace
// ---------------------------------------------------------------------------

fn extract_package(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    let name = build_qualified_name(node, src);
    if name.is_empty() {
        return;
    }
    let line = node.start_position().row as u32;

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Namespace,
        visibility: None,
        start_line: line,
        end_line: line,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(format!("package {}", name)),
        doc_comment: None,
        scope_path: None,
        parent_index,
            byte_offset: 0,
    declared_type: None,
    return_type: None,
    param_types: Vec::new(),
    generic_params: Vec::new(),
});
}

// ---------------------------------------------------------------------------
// Class extraction
// ---------------------------------------------------------------------------

fn extract_class(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    namespace: Option<&str>,
) {
    // class_declaration has a `name` field (identifier)
    let name = match named_field_text(node, "name", src) {
        Some(n) => n,
        None => return,
    };

    // Qualify the class name with the package namespace so that the
    // inherits_map (keyed by qname) and scope_path can be resolved correctly.
    // e.g. `class Foo` in `package org.example` → qname = "org.example.Foo"
    let class_qname = match namespace {
        Some(ns) => format!("{}.{}", ns, name),
        None => name.clone(),
    };

    let line = node.start_position().row as u32;
    let class_idx = symbols.len();

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: class_qname.clone(),
        kind: SymbolKind::Class,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(format!("class {} {{ ... }}", name)),
        doc_comment: None,
        scope_path: None,
        parent_index,
            byte_offset: 0,
    declared_type: None,
    return_type: None,
    param_types: Vec::new(),
    generic_params: Vec::new(),
});

    // Extract superclass (extends) → Inherits edge
    if let Some(superclass_node) = node.child_by_field_name("superclass") {
        let mut sc = superclass_node.walk();
        for sc_child in superclass_node.children(&mut sc) {
            if sc_child.kind() == "type_identifier" || sc_child.kind() == "identifier" {
                let target = node_text(&sc_child, src).to_string();
                if !target.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index: class_idx,
                        target_name: target,
                        kind: EdgeKind::Inherits,
                        line: superclass_node.start_position().row as u32,
                        col: 0,
                        module: None,
                        chain: None,
                        byte_offset: superclass_node.start_byte() as u32,
                                            namespace_segments: Vec::new(),
                                            call_args: Vec::new(),
});
                }
            }
        }
    }

    // Extract interfaces (implements) → Implements edges
    if let Some(interfaces_node) = node.child_by_field_name("interfaces") {
        extract_type_list_refs(&interfaces_node, src, class_idx, EdgeKind::Implements, refs);
    }

    // Walk class body for methods, fields, and nested classes.
    // Pass the class's qualified name as `class_scope` so methods can set
    // scope_path correctly — enabling the inheritance-chain resolver to
    // find the calling class when looking up `{ancestor}.{method_name}`.
    let class_scope = Some(class_qname.as_str());
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            match child.kind() {
                "method_declaration" => {
                    extract_method_declaration(&child, src, symbols, refs, Some(class_idx), class_scope);
                }
                "function_definition" => {
                    extract_function(&child, src, symbols, refs, Some(class_idx), true, class_scope);
                }
                "class_declaration" => {
                    // Inner / nested class — recurse so its methods are found.
                    extract_class(&child, src, symbols, refs, Some(class_idx), namespace);
                }
                "interface_declaration" => {
                    // Nested interface — extract so implementations resolve.
                    extract_interface(&child, src, symbols, refs, Some(class_idx), namespace);
                }
                "field_declaration" => {
                    extract_field(&child, src, symbols, Some(class_idx));
                }
                "method_invocation" => {
                    let local_types = HashMap::new();
                    extract_call(&child, src, class_idx, refs, &local_types);
                }
                _ => {
                    let local_types = HashMap::new();
                    visit_for_calls(&child, src, class_idx, refs, &local_types);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Interface extraction (interface_declaration)
// ---------------------------------------------------------------------------

fn extract_interface(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    namespace: Option<&str>,
) {
    let name = match named_field_text(node, "name", src) {
        Some(n) => n,
        None => return,
    };

    let iface_qname = match namespace {
        Some(ns) => format!("{}.{}", ns, name),
        None => name.clone(),
    };

    let line = node.start_position().row as u32;
    let iface_idx = symbols.len();

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: iface_qname.clone(),
        kind: SymbolKind::Class,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(format!("interface {} {{ ... }}", name)),
        doc_comment: None,
        scope_path: None,
        parent_index,
            byte_offset: 0,
    declared_type: None,
    return_type: None,
    param_types: Vec::new(),
    generic_params: Vec::new(),
});

    // Extract parent interfaces (extends_interfaces child → type_list)
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "extends_interfaces" {
            extract_type_list_refs(&child, src, iface_idx, EdgeKind::Inherits, refs);
        }
    }
}

// ---------------------------------------------------------------------------
// Field extraction (class body `field_declaration`)
// ---------------------------------------------------------------------------

fn extract_field(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    let type_name = named_field_text(node, "type", src).unwrap_or_default();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            let field_name = match named_field_text(&child, "name", src) {
                Some(n) => n,
                None => continue,
            };
            let line = child.start_position().row as u32;
            let sig = if type_name.is_empty() {
                field_name.clone()
            } else {
                format!("{} {}", type_name, field_name)
            };
            symbols.push(ExtractedSymbol {
                name: field_name.clone(),
                qualified_name: field_name.clone(),
                kind: SymbolKind::Field,
                visibility: Some(Visibility::Public),
                start_line: line,
                end_line: child.end_position().row as u32,
                start_col: child.start_position().column as u32,
                end_col: 0,
                signature: Some(sig),
                doc_comment: None,
                scope_path: None,
                parent_index,
                            byte_offset: 0,
    declared_type: None,
    return_type: None,
    param_types: Vec::new(),
    generic_params: Vec::new(),
});
        }
    }
}

// ---------------------------------------------------------------------------
// Type list helper — walks super_interfaces / type_list for Inherits/Implements
// ---------------------------------------------------------------------------

fn extract_type_list_refs(
    node: &Node,
    src: &str,
    source_idx: usize,
    kind: EdgeKind,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "type_identifier" | "identifier" => {
                let name = node_text(&child, src).to_string();
                if !name.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index: source_idx,
                        target_name: name,
                        kind,
                        line: child.start_position().row as u32,
                        col: 0,
                        module: None,
                        chain: None,
                        byte_offset: child.start_byte() as u32,
                                            namespace_segments: Vec::new(),
                                            call_args: Vec::new(),
});
                }
            }
            _ => {
                extract_type_list_refs(&child, src, source_idx, kind, refs);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Function (top-level `def fn(...)`)
// ---------------------------------------------------------------------------

fn extract_function(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    inside_class: bool,
    class_scope: Option<&str>,
) {
    // function_definition has a `name` field
    let name = match named_field_text(node, "name", src) {
        Some(n) => n,
        None => return,
    };

    let line = node.start_position().row as u32;
    let kind = if inside_class { SymbolKind::Method } else { SymbolKind::Function };
    let idx = symbols.len();

    // Qualify the name when inside a class so the method appears as
    // `org.example.MyClass.myMethod` in the index.
    let qualified_name = match class_scope {
        Some(cls) => format!("{}.{}", cls, name),
        None => name.clone(),
    };

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(format!("def {}", name)),
        doc_comment: None,
        // scope_path = class qname so the inheritance-chain resolver can find
        // the enclosing class for bare method calls like `assertSingleViolation()`.
        scope_path: class_scope.map(|s| s.to_string()),
        parent_index,
            byte_offset: 0,
    declared_type: None,
    return_type: None,
    param_types: Vec::new(),
    generic_params: Vec::new(),
});

    let local_types = scan_local_types(node, src);
    emit_local_variable_symbols(node, src, idx, symbols);
    visit_for_calls(node, src, idx, refs, &local_types);
}

// ---------------------------------------------------------------------------
// Method (typed form: `int add(int a, int b)`)
// ---------------------------------------------------------------------------

fn extract_method_declaration(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    class_scope: Option<&str>,
) {
    // method_declaration has fields: type, name, parameters, body
    let name = match named_field_text(node, "name", src) {
        Some(n) => n,
        None => return,
    };

    let return_type = named_field_text(node, "type", src).unwrap_or_default();
    let line = node.start_position().row as u32;
    let idx = symbols.len();

    let sig = if return_type.is_empty() {
        name.clone()
    } else {
        format!("{} {}", return_type, name)
    };

    // Qualify the name when inside a class.
    let qualified_name = match class_scope {
        Some(cls) => format!("{}.{}", cls, name),
        None => name.clone(),
    };

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Method,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(sig),
        doc_comment: None,
        // scope_path = class qname so the inheritance-chain resolver can find
        // the enclosing class for bare method calls.
        scope_path: class_scope.map(|s| s.to_string()),
        parent_index,
            byte_offset: 0,
    declared_type: None,
    return_type: None,
    param_types: Vec::new(),
    generic_params: Vec::new(),
});

    let local_types = scan_local_types(node, src);
    emit_local_variable_symbols(node, src, idx, symbols);
    visit_for_calls(node, src, idx, refs, &local_types);
}

// ---------------------------------------------------------------------------
// Local variable symbol emission (flow engine LHS correlation)
// ---------------------------------------------------------------------------

/// Walk a method/function body and emit a Variable symbol for each
/// `local_variable_declaration` declarator.  These symbols are not indexed
/// for search — they exist so the flow engine can correlate assignment LHS
/// names to indices and bind the RHS call's return type to the local.
fn emit_local_variable_symbols(
    body_root: &Node,
    src: &str,
    parent_index: usize,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let mut cursor = body_root.walk();
    for child in body_root.children(&mut cursor) {
        if child.kind() == "local_variable_declaration" {
            let mut dc = child.walk();
            for decl in child.children(&mut dc) {
                if decl.kind() == "variable_declarator" {
                    if let Some(name_node) = decl.child_by_field_name("name") {
                        let name = node_text(&name_node, src).to_string();
                        if !name.is_empty() {
                            symbols.push(ExtractedSymbol {
                                name: name.clone(),
                                qualified_name: name,
                                kind: SymbolKind::Variable,
                                visibility: None,
                                start_line: name_node.start_position().row as u32,
                                end_line: name_node.end_position().row as u32,
                                start_col: name_node.start_position().column as u32,
                                end_col: name_node.end_position().column as u32,
                                signature: None,
                                doc_comment: None,
                                scope_path: None,
                                parent_index: Some(parent_index),
                                byte_offset: 0,
                                                            declared_type: None,
                                return_type: None,
                                param_types: Vec::new(),
                                generic_params: Vec::new(),
});
                        }
                    }
                }
            }
        }
        emit_local_variable_symbols(&child, src, parent_index, symbols);
    }
}

// ---------------------------------------------------------------------------
// Import extraction
// ---------------------------------------------------------------------------

fn extract_import(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let text = node_text(node, src);
    // Strip `import ` prefix and any `as Alias` suffix.
    // Also strip the optional `static` keyword that appears in static imports:
    //   import static org.codenarc.test.TestUtil.shouldFail
    // After stripping "import" we may see "static" as the next token — skip it.
    let after_import = text
        .trim_start_matches("import")
        .trim();

    // Skip the `static` keyword when present.
    let path_str = if after_import.starts_with("static ") || after_import == "static" {
        after_import.trim_start_matches("static").trim()
    } else {
        after_import
    };

    let full_path = path_str
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_end_matches('*')
        .trim_end_matches('.')
        .to_string();

    if full_path.is_empty() {
        return;
    }

    // For a static member import `import static pkg.Class.member`, the target_name
    // is the simple member name (last segment) so the Java resolver's exact-import
    // lookup (which matches `imported_name == effective_target`) can find it.
    // The `module` carries the full qualified path so `by_qualified_name` works.
    let is_static_import = after_import.starts_with("static ");
    let (target_name, module_path) = if is_static_import {
        let simple = full_path
            .rfind('.')
            .map(|i| full_path[i + 1..].to_string())
            .unwrap_or_else(|| full_path.clone());
        (simple, full_path.clone())
    } else {
        (full_path.clone(), full_path.clone())
    };

    refs.push(ExtractedRef {
        source_symbol_index,
        target_name,
        kind: EdgeKind::Imports,
        line: node.start_position().row as u32,
        col: 0,
        module: Some(module_path),
        chain: None,
        byte_offset: node.start_byte() as u32,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
});
}

// ---------------------------------------------------------------------------
// Call extraction (method_invocation)
// ---------------------------------------------------------------------------

pub(super) fn extract_call(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
    local_types: &HashMap<String, String>,
) {
    // method_invocation has field `name` (identifier)
    let name = match named_field_text(node, "name", src) {
        Some(n) => n,
        None => return,
    };

    // Skip control flow keywords that the grammar sometimes parses as method_invocation.
    if predicates::is_groovy_keyword(&name) {
        return;
    }

    // Build a MemberChain when the call has a receiver (`object` field).
    // This enables the chain walker and the external classifier to determine
    // the receiver type and classify the call correctly (e.g. `file.path.endsWith`
    // where `file` has declared type `File` from a for-loop or local declaration).
    let chain = node.child_by_field_name("object")
        .and_then(|obj| build_receiver_chain(&obj, &name, src, local_types));

    refs.push(ExtractedRef {
        source_symbol_index,
        target_name: name,
        kind: EdgeKind::Calls,
        line: node.start_position().row as u32,
        col: 0,
        module: None,
        chain,
        byte_offset: node.start_byte() as u32,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    });
}
