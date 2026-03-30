// =============================================================================
// ruby/symbols.rs  —  Symbol pushers for Ruby
// =============================================================================

use super::calls::extract_calls_from_body_with_symbols;
use super::helpers::{
    build_method_signature, get_call_method_name, node_text, qualify, ruby_visibility,
    scope_from_prefix,
};
use super::params::{extract_method_params, extract_rescue};
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// Rails and ActiveRecord macros that produce TypeRef edges
// ---------------------------------------------------------------------------

pub(super) static RAILS_ASSOC_MACROS: &[&str] =
    &["has_many", "belongs_to", "has_one", "has_and_belongs_to_many", "through"];

// attr_* macros that produce Property symbols
pub(super) static ATTR_MACROS: &[&str] = &["attr_reader", "attr_writer", "attr_accessor"];

// ---------------------------------------------------------------------------
// Class
// ---------------------------------------------------------------------------

pub(super) fn extract_class(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let name = node_text(&name_node, src);
    let qualified_name = qualify(&name, qualified_prefix);
    let new_prefix = qualified_name.clone();

    let signature = {
        let raw = node_text(node, src);
        raw.lines().next().map(|l| l.trim().to_string())
    };

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Class,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature,
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });

    // Inheritance: `class Foo < Bar`
    if let Some(superclass_node) = node.child_by_field_name("superclass") {
        let super_name = {
            let mut found: Option<String> = None;
            let mut sc = superclass_node.walk();
            for c in superclass_node.children(&mut sc) {
                if c.kind() == "constant" || c.kind() == "scope_resolution" {
                    found = Some(node_text(&c, src));
                    break;
                }
            }
            found.unwrap_or_else(|| {
                let raw = node_text(&superclass_node, src);
                raw.trim_start_matches('<').trim().to_string()
            })
        };
        refs.push(ExtractedRef {
            source_symbol_index: idx,
            target_name: super_name,
            kind: EdgeKind::Inherits,
            line: superclass_node.start_position().row as u32,
            module: None,
            chain: None,
        });
    }

    if let Some(body) = node.child_by_field_name("body") {
        super::extract_from_node(body, src, symbols, refs, Some(idx), &new_prefix, true);
    }
}

// ---------------------------------------------------------------------------
// Module
// ---------------------------------------------------------------------------

pub(super) fn extract_module(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let name = node_text(&name_node, src);
    let qualified_name = qualify(&name, qualified_prefix);
    let new_prefix = qualified_name.clone();

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        // Ruby modules map to Interface — they define a contract/namespace, no state.
        kind: SymbolKind::Interface,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("module {name}")),
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });

    if let Some(body) = node.child_by_field_name("body") {
        super::extract_from_node(body, src, symbols, refs, Some(idx), &new_prefix, false);
    }
}

// ---------------------------------------------------------------------------
// Method
// ---------------------------------------------------------------------------

pub(super) fn extract_method(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    inside_class: bool,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let name = node_text(&name_node, src);
    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = ruby_visibility(&name);

    let kind = if name == "initialize" {
        SymbolKind::Constructor
    } else if inside_class {
        SymbolKind::Method
    } else {
        SymbolKind::Function
    };

    let signature = build_method_signature(node, src, &name);

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name,
        qualified_name,
        kind,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature,
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });

    // Extract parameter names as Variable symbols scoped to this method.
    if let Some(params_node) = node.child_by_field_name("parameters") {
        extract_method_params(&params_node, src, idx, qualified_prefix, symbols);
    }

    if let Some(body) = node.child_by_field_name("body") {
        extract_calls_from_body_with_symbols(&body, src, idx, refs, Some(symbols));
        extract_rescue_from_body(&body, src, idx, refs, symbols, qualified_prefix);
    }
}

// ---------------------------------------------------------------------------
// Singleton method (`def self.foo` / `def ClassName.foo`)
// ---------------------------------------------------------------------------

pub(super) fn extract_singleton_method(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let name = node_text(&name_node, src);
    let qualified_name = qualify(&name, qualified_prefix);
    let signature = build_method_signature(node, src, &name);

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::Method,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature,
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });

    // Extract parameter names as Variable symbols.
    if let Some(params_node) = node.child_by_field_name("parameters") {
        extract_method_params(&params_node, src, idx, qualified_prefix, symbols);
    }

    if let Some(body) = node.child_by_field_name("body") {
        extract_calls_from_body_with_symbols(&body, src, idx, refs, Some(symbols));
        extract_rescue_from_body(&body, src, idx, refs, symbols, qualified_prefix);
    }
}

// ---------------------------------------------------------------------------
// Call statements: require, require_relative, attr_*, Rails macros
// ---------------------------------------------------------------------------

pub(super) fn extract_call_statement(
    node: &Node,
    src: &[u8],
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
    parent_index: Option<usize>,
    inside_class: bool,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let method_name = get_call_method_name(node, src);

    match method_name.as_deref() {
        Some("require") | Some("require_relative") => {
            extract_require(node, src, refs, current_symbol_count, method_name.as_deref());
        }

        Some(m) if ATTR_MACROS.contains(&m) => {
            if inside_class {
                extract_attr_macro(node, src, symbols, parent_index, m);
            }
        }

        Some(m) if RAILS_ASSOC_MACROS.contains(&m) => {
            if let Some(args) = node.child_by_field_name("arguments") {
                let mut c = args.walk();
                for arg in args.children(&mut c) {
                    if arg.kind() == "simple_symbol" || arg.kind() == "symbol" {
                        let raw = node_text(&arg, src);
                        let assoc_name = raw.trim_start_matches(':').to_string();
                        refs.push(ExtractedRef {
                            source_symbol_index: current_symbol_count.saturating_sub(1),
                            target_name: assoc_name,
                            kind: EdgeKind::TypeRef,
                            line: arg.start_position().row as u32,
                            module: None,
                            chain: None,
                        });
                        break;
                    }
                }
            }
        }

        _ => {
            if let Some(pidx) = parent_index {
                if let Some(recv) = node.child_by_field_name("receiver") {
                    let recv_text = node_text(&recv, src);
                    if let Some(mname) = method_name.as_deref() {
                        if mname == "new" {
                            refs.push(ExtractedRef {
                                source_symbol_index: pidx,
                                target_name: recv_text,
                                kind: EdgeKind::Instantiates,
                                line: node.start_position().row as u32,
                                module: None,
                                chain: None,
                            });
                        } else {
                            refs.push(ExtractedRef {
                                source_symbol_index: pidx,
                                target_name: mname.to_string(),
                                kind: EdgeKind::Calls,
                                line: node.start_position().row as u32,
                                module: None,
                                chain: None,
                            });
                        }
                    }
                } else if let Some(mname) = method_name.as_deref() {
                    refs.push(ExtractedRef {
                        source_symbol_index: pidx,
                        target_name: mname.to_string(),
                        kind: EdgeKind::Calls,
                        line: node.start_position().row as u32,
                        module: None,
                        chain: None,
                    });
                }
            }
        }
    }
}

fn extract_require(
    node: &Node,
    src: &[u8],
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
    method_name: Option<&str>,
) {
    let is_relative = method_name == Some("require_relative");
    if let Some(args) = node.child_by_field_name("arguments") {
        let mut c = args.walk();
        for arg in args.children(&mut c) {
            let kind = arg.kind();
            if kind == "string" || kind == "string_content" {
                let raw = node_text(&arg, src);
                let path = raw
                    .trim_start_matches('"')
                    .trim_end_matches('"')
                    .trim_start_matches('\'')
                    .trim_end_matches('\'')
                    .to_string();
                let (target, module) = if is_relative {
                    let stem = path.rsplit('/').next().unwrap_or(&path);
                    (stem.to_string(), Some(path))
                } else {
                    let parts: Vec<&str> = path.split('/').collect();
                    let target = parts.last().unwrap_or(&path.as_str()).to_string();
                    let module = if parts.len() > 1 {
                        Some(parts[..parts.len() - 1].join("/"))
                    } else {
                        None
                    };
                    (target, module)
                };
                refs.push(ExtractedRef {
                    source_symbol_index: current_symbol_count,
                    target_name: target,
                    kind: EdgeKind::Imports,
                    line: arg.start_position().row as u32,
                    module,
                    chain: None,
                });
                break;
            }
        }
    }
}

/// Scan a subtree for `rescue` nodes and extract TypeRef/Variable from them.
fn extract_rescue_from_body(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
    symbols: &mut Vec<ExtractedSymbol>,
    qualified_prefix: &str,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "rescue" {
            extract_rescue(&child, src, source_symbol_index, symbols, refs, qualified_prefix);
            extract_rescue_from_body(&child, src, source_symbol_index, refs, symbols, qualified_prefix);
        } else {
            extract_rescue_from_body(&child, src, source_symbol_index, refs, symbols, qualified_prefix);
        }
    }
}

fn extract_attr_macro(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    _macro_name: &str,
) {
    let Some(args) = node.child_by_field_name("arguments") else {
        return;
    };
    let mut c = args.walk();
    for arg in args.children(&mut c) {
        let kind = arg.kind();
        if kind == "simple_symbol" || kind == "symbol" {
            let raw = node_text(&arg, src);
            let name = raw.trim_start_matches(':').to_string();
            let qualified_name = name.clone();
            symbols.push(ExtractedSymbol {
                name,
                qualified_name,
                kind: SymbolKind::Property,
                visibility: Some(Visibility::Public),
                start_line: arg.start_position().row as u32,
                end_line: arg.end_position().row as u32,
                start_col: arg.start_position().column as u32,
                end_col: arg.end_position().column as u32,
                signature: None,
                doc_comment: None,
                scope_path: None,
                parent_index,
            });
        }
    }
}
