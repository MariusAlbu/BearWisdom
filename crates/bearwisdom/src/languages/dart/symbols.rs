// =============================================================================
// dart/symbols.rs  —  Symbol extraction for Dart
// =============================================================================

use super::calls::extract_dart_calls;
use super::helpers::{first_child_text_of_kind, get_field_text, node_text, qualify, scope_from_prefix};
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// Class / Mixin / Extension
// ---------------------------------------------------------------------------

pub(super) fn extract_class(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let name = match get_field_text(node, src, "name")
        .or_else(|| first_child_text_of_kind(node, src, "identifier"))
    {
        Some(n) => n,
        None => return,
    };
    let qualified_name = qualify(&name, qualified_prefix);
    let idx = symbols.len();
    let new_prefix = qualified_name.clone();

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: qualified_name.clone(),
        kind: SymbolKind::Class,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("class {name}")),
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });

    extract_dart_heritage(node, src, idx, refs);

    if let Some(body) = node.child_by_field_name("body") {
        extract_class_body(&body, src, symbols, refs, Some(idx), &new_prefix);
    }
}

pub(super) fn extract_mixin(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let name = match get_field_text(node, src, "name")
        .or_else(|| first_child_text_of_kind(node, src, "identifier"))
    {
        Some(n) => n,
        None => return,
    };
    let qualified_name = qualify(&name, qualified_prefix);
    let idx = symbols.len();
    let new_prefix = qualified_name.clone();

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Class,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("mixin {name}")),
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });

    if let Some(body) = node.child_by_field_name("body") {
        extract_class_body(&body, src, symbols, refs, Some(idx), &new_prefix);
    }
}

pub(super) fn extract_extension(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let name = get_field_text(node, src, "name")
        .or_else(|| first_child_text_of_kind(node, src, "identifier"))
        .unwrap_or_else(|| "<extension>".to_string());
    let qualified_name = qualify(&name, qualified_prefix);
    let idx = symbols.len();
    let new_prefix = qualified_name.clone();

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Interface,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("extension {name}")),
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });

    if let Some(body) = node.child_by_field_name("body") {
        extract_class_body(&body, src, symbols, refs, Some(idx), &new_prefix);
    }
}

pub(super) fn extract_enum(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let name = match get_field_text(node, src, "name")
        .or_else(|| first_child_text_of_kind(node, src, "identifier"))
    {
        Some(n) => n,
        None => return,
    };
    let qualified_name = qualify(&name, qualified_prefix);
    let idx = symbols.len();

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: qualified_name.clone(),
        kind: SymbolKind::Enum,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("enum {name}")),
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "enum_constant" {
            let member_name = get_field_text(&child, src, "name")
                .or_else(|| first_child_text_of_kind(&child, src, "identifier"))
                .unwrap_or_default();
            if member_name.is_empty() {
                continue;
            }
            symbols.push(ExtractedSymbol {
                name: member_name.clone(),
                qualified_name: format!("{qualified_name}.{member_name}"),
                kind: SymbolKind::EnumMember,
                visibility: Some(Visibility::Public),
                start_line: child.start_position().row as u32,
                end_line: child.end_position().row as u32,
                start_col: child.start_position().column as u32,
                end_col: child.end_position().column as u32,
                signature: None,
                doc_comment: None,
                scope_path: Some(qualified_name.clone()),
                parent_index: Some(idx),
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Class body
// ---------------------------------------------------------------------------

pub(super) fn extract_class_body(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "method_signature" | "function_signature" => {
                extract_method(&child, src, symbols, refs, parent_index, qualified_prefix);
            }
            "constructor_signature" => {
                extract_constructor(&child, src, symbols, parent_index, qualified_prefix);
            }
            "field_declaration" | "initialized_variable_definition" => {
                extract_field(&child, src, symbols, parent_index, qualified_prefix);
            }
            "getter_signature" | "setter_signature" => {
                extract_getter_setter(&child, src, symbols, refs, parent_index, qualified_prefix);
            }
            "static_final_declaration_list" | "declaration" => {
                extract_class_body(&child, src, symbols, refs, parent_index, qualified_prefix);
            }
            _ => {
                extract_class_body(&child, src, symbols, refs, parent_index, qualified_prefix);
            }
        }
    }
}

fn extract_method(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    // For getter/setter signatures wrapped in method_signature, delegate.
    let mut cursor_check = node.walk();
    for child in node.named_children(&mut cursor_check) {
        if child.kind() == "getter_signature" || child.kind() == "setter_signature" {
            extract_getter_setter(&child, src, symbols, refs, parent_index, qualified_prefix);
            return;
        }
    }

    let name = match get_field_text(node, src, "name")
        .or_else(|| first_child_text_of_kind(node, src, "identifier"))
    {
        Some(n) => n,
        None => return,
    };

    let qualified_name = qualify(&name, qualified_prefix);
    let idx = symbols.len();

    let visibility = if name.starts_with('_') {
        Some(Visibility::Private)
    } else {
        Some(Visibility::Public)
    };

    symbols.push(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::Method,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: None,
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });

    if let Some(body) = node.next_sibling() {
        if body.kind() == "function_body" || body.kind() == "block" {
            extract_dart_calls(&body, src, idx, refs);
        }
    }
}

fn extract_constructor(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let raw_name = node_text(*node, src);
    let ctor_name = raw_name
        .find('(')
        .map(|i| raw_name[..i].trim().to_string())
        .unwrap_or_else(|| raw_name.trim().to_string());
    let simple = ctor_name
        .rsplit('.')
        .next()
        .unwrap_or(&ctor_name)
        .to_string();

    let qualified_name = qualify(&simple, qualified_prefix);
    let visibility = if simple.starts_with('_') {
        Some(Visibility::Private)
    } else {
        Some(Visibility::Public)
    };

    symbols.push(ExtractedSymbol {
        name: simple,
        qualified_name,
        kind: SymbolKind::Constructor,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(ctor_name),
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });
}

fn extract_field(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = child.kind();
        if kind == "initialized_identifier" || kind == "identifier" {
            let name = if kind == "initialized_identifier" {
                get_field_text(&child, src, "name")
                    .or_else(|| first_child_text_of_kind(&child, src, "identifier"))
                    .unwrap_or_default()
            } else {
                node_text(child, src)
            };

            if name.is_empty() || name == "final" || name == "static" || name == "late" {
                continue;
            }

            let qualified_name = qualify(&name, qualified_prefix);
            let visibility = if name.starts_with('_') {
                Some(Visibility::Private)
            } else {
                Some(Visibility::Public)
            };

            symbols.push(ExtractedSymbol {
                name,
                qualified_name,
                kind: SymbolKind::Property,
                visibility,
                start_line: child.start_position().row as u32,
                end_line: child.end_position().row as u32,
                start_col: child.start_position().column as u32,
                end_col: child.end_position().column as u32,
                signature: None,
                doc_comment: None,
                scope_path: scope_from_prefix(qualified_prefix),
                parent_index,
            });
        }
    }
}

pub(super) fn extract_top_level_function(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let name = match get_field_text(node, src, "name")
        .or_else(|| first_child_text_of_kind(node, src, "identifier"))
    {
        Some(n) => n,
        None => return,
    };
    let qualified_name = qualify(&name, qualified_prefix);
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("{name}()")),
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });
}

pub(super) fn extract_variable(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let name = match get_field_text(node, src, "name")
        .or_else(|| first_child_text_of_kind(node, src, "identifier"))
    {
        Some(n) => n,
        None => return,
    };
    let qualified_name = qualify(&name, qualified_prefix);
    symbols.push(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::Variable,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: None,
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });
}

// ---------------------------------------------------------------------------
// Import / part directives
// ---------------------------------------------------------------------------

pub(super) fn extract_import_directive(
    node: &Node,
    src: &str,
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    extract_import_spec_recursive(node, src, current_symbol_count, refs);
}

fn extract_import_spec_recursive(
    node: &Node,
    src: &str,
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let k = node.kind();
    if k == "import_specification" || k == "library_import" || k == "import_or_export" {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let ck = child.kind();
            if ck == "string_literal" || ck == "uri" || ck == "configurable_uri" {
                let raw = if ck == "configurable_uri" {
                    first_child_text_of_kind(&child, src, "string_literal")
                        .unwrap_or_else(|| node_text(child, src))
                } else {
                    node_text(child, src)
                };
                let module = raw.trim_matches('"').trim_matches('\'').to_string();
                let target = module
                    .rsplit('/')
                    .next()
                    .unwrap_or(&module)
                    .trim_end_matches(".dart")
                    .to_string();
                refs.push(ExtractedRef {
                    source_symbol_index: current_symbol_count,
                    target_name: target,
                    kind: EdgeKind::Imports,
                    line: child.start_position().row as u32,
                    module: Some(module),
                    chain: None,
                });
            } else if ck == "import_specification" || ck == "library_import" {
                extract_import_spec_recursive(&child, src, current_symbol_count, refs);
            }
        }
    }
}

pub(super) fn extract_part_directive(
    node: &Node,
    src: &str,
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "string_literal" || child.kind() == "uri" {
            let raw = node_text(child, src);
            let module = raw.trim_matches('"').trim_matches('\'').to_string();
            let target = module
                .rsplit('/')
                .next()
                .unwrap_or(&module)
                .trim_end_matches(".dart")
                .to_string();
            refs.push(ExtractedRef {
                source_symbol_index: current_symbol_count,
                target_name: target,
                kind: EdgeKind::Imports,
                line: child.start_position().row as u32,
                module: Some(module),
                chain: None,
            });
        }
    }
}

/// Emit a TypeAlias symbol for a Dart `typedef` / `type_alias` declaration.
/// The grammar's `type_alias` has no `name` field — the name is a
/// `type_identifier` child before `=`.
pub(super) fn extract_typedef(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    // Walk children: take the first `type_identifier` as the alias name.
    let name = {
        let mut found: Option<String> = None;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "type_identifier" || child.kind() == "identifier" {
                found = Some(node_text(child, src));
                break;
            }
        }
        match found {
            Some(n) => n,
            None => return,
        }
    };

    let qualified_name = qualify(&name, qualified_prefix);
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::TypeAlias,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("typedef {name}")),
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });
}

/// Emit Method symbols for `getter_signature` and `setter_signature` nodes.
pub(super) fn extract_getter_setter(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let name = match get_field_text(node, src, "name")
        .or_else(|| first_child_text_of_kind(node, src, "identifier"))
    {
        Some(n) => n,
        None => return,
    };
    let qualified_name = qualify(&name, qualified_prefix);
    let idx = symbols.len();

    let visibility = if name.starts_with('_') {
        Some(Visibility::Private)
    } else {
        Some(Visibility::Public)
    };

    let is_getter = node.kind() == "getter_signature";
    let sig_prefix = if is_getter { "get" } else { "set" };

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Method,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("{sig_prefix} {name}")),
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });

    // Extract calls from the body (sibling node).
    if let Some(body) = node.next_sibling() {
        if body.kind() == "function_body" || body.kind() == "block" {
            extract_dart_calls(&body, src, idx, refs);
        }
    }
}

// ---------------------------------------------------------------------------
// Heritage
// ---------------------------------------------------------------------------

pub(super) fn extract_dart_heritage(
    node: &Node,
    src: &str,
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "superclass" => {
                let mut c = child.walk();
                for n in child.children(&mut c) {
                    if n.kind() == "type_name" || n.kind() == "identifier" {
                        refs.push(ExtractedRef {
                            source_symbol_index: source_idx,
                            target_name: node_text(n, src),
                            kind: EdgeKind::Inherits,
                            line: n.start_position().row as u32,
                            module: None,
                            chain: None,
                        });
                    }
                }
            }
            "interfaces" => {
                let mut c = child.walk();
                for n in child.children(&mut c) {
                    if n.kind() == "type_name" || n.kind() == "identifier" {
                        refs.push(ExtractedRef {
                            source_symbol_index: source_idx,
                            target_name: node_text(n, src),
                            kind: EdgeKind::Implements,
                            line: n.start_position().row as u32,
                            module: None,
                            chain: None,
                        });
                    }
                }
            }
            "mixins" | "mixin_application" => {
                let mut c = child.walk();
                for n in child.children(&mut c) {
                    if n.kind() == "type_name" || n.kind() == "identifier" {
                        refs.push(ExtractedRef {
                            source_symbol_index: source_idx,
                            target_name: node_text(n, src),
                            kind: EdgeKind::TypeRef,
                            line: n.start_position().row as u32,
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
