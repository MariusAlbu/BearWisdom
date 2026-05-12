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

    // Enum constants live inside an `enum_body` child (Dart grammar 0.1).
    // Walk the direct children first; if we find an enum_body, recurse into it.
    let extract_constant = |child: &tree_sitter::Node, symbols: &mut Vec<ExtractedSymbol>| {
        let member_name = get_field_text(child, src, "name")
            .or_else(|| first_child_text_of_kind(child, src, "identifier"))
            .unwrap_or_default();
        if member_name.is_empty() {
            return;
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
    };

    // Check both direct children and those inside `enum_body`.
    let mut outer = node.walk();
    for child in node.children(&mut outer) {
        if child.kind() == "enum_constant" {
            extract_constant(&child, symbols);
        } else if child.kind() == "enum_body" {
            let mut inner_cursor = child.walk();
            for item in child.children(&mut inner_cursor) {
                if item.kind() == "enum_constant" {
                    extract_constant(&item, symbols);
                }
            }
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
            "method_signature" => {
                // A method_signature may wrap a factory_constructor_signature, constructor_signature,
                // getter/setter, or regular function_signature.  Walk named children to decide.
                let mut handled = false;
                let mut mc = child.walk();
                for inner in child.named_children(&mut mc) {
                    match inner.kind() {
                        "factory_constructor_signature" | "redirecting_factory_constructor_signature" => {
                            extract_factory_constructor(&inner, src, symbols, parent_index, qualified_prefix);
                            handled = true;
                            break;
                        }
                        "constructor_signature" => {
                            extract_constructor(&inner, src, symbols, parent_index, qualified_prefix);
                            handled = true;
                            break;
                        }
                        _ => {}
                    }
                }
                if !handled {
                    extract_method(&child, src, symbols, refs, parent_index, qualified_prefix);
                }
            }
            "function_signature" => {
                extract_method(&child, src, symbols, refs, parent_index, qualified_prefix);
            }
            "constructor_signature" => {
                extract_constructor(&child, src, symbols, parent_index, qualified_prefix);
            }
            // `factory ClassName(...)` — emit as Constructor symbol.
            "factory_constructor_signature" | "redirecting_factory_constructor_signature" => {
                extract_factory_constructor(&child, src, symbols, parent_index, qualified_prefix);
            }
            "field_declaration" | "initialized_variable_definition" => {
                let pre_len = symbols.len();
                extract_field(&child, src, symbols, parent_index, qualified_prefix);
                // Emit TypeRef for the field's type annotation by routing through
                // extract_dart_calls which handles type_identifier at every level.
                let sym_idx = if symbols.len() > pre_len { pre_len } else { parent_index.unwrap_or(0) };
                extract_dart_calls(&child, src, sym_idx, refs);
                // Type inference from initializer: if no explicit type annotation,
                // infer from constructor call. `final repo = Repository();` → TypeRef "Repository".
                if !has_type_annotation(&child) {
                    infer_type_from_dart_initializer(&child, src, sym_idx, refs);
                }
            }
            "getter_signature" | "setter_signature" => {
                extract_getter_setter(&child, src, symbols, refs, parent_index, qualified_prefix);
            }
            "static_final_declaration_list" | "declaration" => {
                // A `declaration` may contain `type_identifier` (the field/variable
                // declared type) plus `initialized_identifier_list` or
                // `function_signature` etc.  Route through extract_dart_calls
                // first to capture type refs, then recurse for symbols.
                let sym_idx = parent_index.unwrap_or(0);
                extract_dart_calls(&child, src, sym_idx, refs);
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

    // `method_signature` wraps a `function_signature` child in Dart grammar 0.1.
    // The name field lives on `function_signature`, not on `method_signature`.
    // For `method_signature`, delegate name lookup to its `function_signature` child.
    let sig_node: Node = if node.kind() == "method_signature" {
        let mut found: Option<Node> = None;
        let mut c = node.walk();
        for child in node.named_children(&mut c) {
            if child.kind() == "function_signature" {
                found = Some(child);
                break;
            }
        }
        match found {
            Some(fs) => fs,
            None => *node,  // Fallback to method_signature itself
        }
    } else {
        *node
    };

    let name = match get_field_text(&sig_node, src, "name")
        .or_else(|| first_child_text_of_kind(&sig_node, src, "identifier"))
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

    // The function body is the next sibling of `node` (method_signature) within class_member.
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
    if k == "import_specification" || k == "library_import" || k == "import_or_export" || k == "library_export" {
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
                    byte_offset: 0,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
});
            } else if ck == "import_specification" || ck == "library_import" || ck == "library_export" {
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
                byte_offset: 0,
                            namespace_segments: Vec::new(),
                            call_args: Vec::new(),
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

/// Emit a Constructor symbol for a `factory_constructor_signature` node.
/// Dart: `factory ClassName.namedCtor(params) => ...`
fn extract_factory_constructor(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    // The constructor name is a `type_identifier` or `qualified_name` child.
    // For named constructors: `ClassName.namedCtor` — take the last identifier.
    let mut name: Option<String> = None;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "type_identifier" | "identifier" => {
                let t = node_text(child, src);
                if !t.is_empty() && t != "factory" {
                    name = Some(t);
                    break;
                }
            }
            "qualified_name" => {
                // `ClassName.namedCtor` — take the last identifier.
                let mut last: Option<String> = None;
                let mut qc = child.walk();
                for inner in child.children(&mut qc) {
                    if inner.kind() == "identifier" || inner.kind() == "type_identifier" {
                        last = Some(node_text(inner, src));
                    }
                }
                if let Some(n) = last {
                    name = Some(n);
                    break;
                }
            }
            _ => {}
        }
    }
    let name = match name {
        Some(n) => n,
        None => return,
    };
    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = if name.starts_with('_') {
        Some(Visibility::Private)
    } else {
        Some(Visibility::Public)
    };
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Constructor,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("factory {name}")),
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });
}

/// Emit TypeRef edges for the declared type of a field declaration.
/// Handles `UserService service;` where `UserService` is a `type_identifier`.
/// The type node may be wrapped in `type_not_void`, `declared_type`, etc.
fn emit_field_type_refs(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    use super::calls::emit_dart_type_ref;
    // Recursively walk the field declaration looking for the first type_identifier
    // that is NOT a keyword or the variable name.
    emit_field_type_refs_inner(node, src, source_symbol_index, refs, &mut false);

    fn emit_field_type_refs_inner(
        node: &tree_sitter::Node,
        src: &str,
        source_symbol_index: usize,
        refs: &mut Vec<ExtractedRef>,
        found: &mut bool,
    ) {
        if *found { return; }
        match node.kind() {
            "type_identifier" => {
                let name = node_text(*node, src);
                if !name.is_empty() && !matches!(name.as_str(), "final" | "static" | "late" | "const" | "var" | "void") {
                    emit_dart_type_ref(*node, src, source_symbol_index, refs);
                    *found = true;
                }
            }
            // Stop recursing into these — they are the variable name/initializer, not the type.
            "initialized_identifier" | "initialized_identifier_list" | "identifier" => {
                // Don't recurse further — identifiers here are variable names.
            }
            _ => {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if *found { break; }
                    emit_field_type_refs_inner(&child, src, source_symbol_index, refs, found);
                }
            }
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
                    source_symbol_index: source_idx,
                    target_name: name,
                    kind: EdgeKind::Inherits,
                    line: type_node.start_position().row as u32,
                    module: None,
                    chain: None,
                    byte_offset: 0,
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
                        source_symbol_index: source_idx,
                        target_name: node_text(n, src),
                        kind: EdgeKind::Inherits,
                        line: n.start_position().row as u32,
                        module: None,
                        chain: None,
                        byte_offset: 0,
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
                    source_symbol_index: source_idx,
                    target_name: node_text(n, src),
                    kind: EdgeKind::Implements,
                    line: n.start_position().row as u32,
                    module: None,
                    chain: None,
                    byte_offset: 0,
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
                    source_symbol_index: source_idx,
                    target_name: node_text(n, src),
                    kind: EdgeKind::TypeRef,
                    line: n.start_position().row as u32,
                    module: None,
                    chain: None,
                    byte_offset: 0,
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

/// Check if a Dart field/variable node has an explicit type annotation.
/// Returns true if a `type_identifier` (or similar type node) appears
/// before the variable name.
fn has_type_annotation(node: &Node) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "type_identifier" | "generic_type" | "function_type" => return true,
            "initialized_identifier_list" | "initialized_identifier" | "=" => break,
            _ => {}
        }
    }
    false
}

/// Infer the type of a Dart field from its initializer expression when no
/// explicit type annotation is present. Emits a TypeRef that feeds `field_type_name`.
///
/// Handles:
///   `final repo = Repository();` → TypeRef to "Repository"
///   `var service = ServiceImpl(config);` → TypeRef to "ServiceImpl"
fn infer_type_from_dart_initializer(
    node: &Node,
    src: &str,
    sym_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Find the initializer expression — typically after `=` inside
    // initialized_variable_definition or directly as the value field.
    let mut cursor = node.walk();
    let mut found_eq = false;
    for child in node.children(&mut cursor) {
        if child.kind() == "=" {
            found_eq = true;
            continue;
        }
        if !found_eq || !child.is_named() {
            continue;
        }
        // Look for constructor invocation patterns.
        match child.kind() {
            // `Repository()` or `SomeType.named()`
            "identifier" => {
                let name = node_text(child, src);
                if name.starts_with(|c: char| c.is_uppercase()) {
                    refs.push(ExtractedRef {
                        source_symbol_index: sym_idx,
                        target_name: name,
                        kind: EdgeKind::TypeRef,
                        line: child.start_position().row as u32,
                        module: None,
                        chain: None,
                        byte_offset: 0,
                                            namespace_segments: Vec::new(),
                                            call_args: Vec::new(),
});
                }
            }
            _ => {
                // Walk deeper for call expressions in the initializer.
                let mut ic = child.walk();
                for inner in child.children(&mut ic) {
                    if inner.kind() == "identifier" {
                        let name = node_text(inner, src);
                        if name.starts_with(|c: char| c.is_uppercase()) {
                            refs.push(ExtractedRef {
                                source_symbol_index: sym_idx,
                                target_name: name,
                                kind: EdgeKind::TypeRef,
                                line: inner.start_position().row as u32,
                                module: None,
                                chain: None,
                                byte_offset: 0,
                                                            namespace_segments: Vec::new(),
                                                            call_args: Vec::new(),
});
                            return;
                        }
                    }
                }
            }
        }
        break;
    }
}
