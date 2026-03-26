// =============================================================================
// parser/extractors/swift.rs  —  Swift symbol and reference extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Class          — `class Foo { … }`
//   Struct         — `struct Foo { … }`
//   Enum           — `enum Foo { … }`
//   EnumMember     — enum `case` declarations
//   Interface      — `protocol Foo { … }`
//   Namespace      — `extension Foo` (treated as Namespace; adds members to an existing type)
//   Method         — `func foo(…)` inside a type
//   Function       — `func foo(…)` at top level
//   Constructor    — `init(…)` / `init?(…)` / `init!(…)`
//   Property       — `let`/`var` stored property declarations
//
// REFERENCES:
//   import_declaration            → EdgeKind::Imports
//   Protocol conformance (`: P`) → EdgeKind::Implements
//   Inheritance (`: Base`)        → EdgeKind::Inherits
//   call_expression               → EdgeKind::Calls
//
// Grammar notes (tree-sitter-swift 0.7):
//   class_declaration         → name: type_identifier, type_inheritance_clause?,
//                               class_body
//   struct_declaration        → name: type_identifier, type_inheritance_clause?,
//                               struct_body
//   enum_declaration          → name: type_identifier, type_inheritance_clause?,
//                               enum_body
//   protocol_declaration      → name: type_identifier, type_inheritance_clause?,
//                               protocol_body
//   extension_declaration     → extended_type child (no name field in all versions)
//   function_declaration      → name: simple_identifier, parameter_clause?,
//                               function_return_type?, function_body?
//   initializer_declaration   → init keyword, parameter_clause?, code_block?
//   property_declaration      → name: simple_identifier (via pattern), type_annotation?
//   import_declaration        → import_kind?, import_path_component+
//   type_inheritance_clause   → inherited_type* (comma-separated)
//   inherited_type            → user_type
//   call_expression           → called_value (the callee), arguments?
//   enum_entry                → name: simple_identifier, associated_values?
// =============================================================================

use crate::parser::scope_tree::{self, ScopeKind};
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Scope configuration
// ---------------------------------------------------------------------------

// tree-sitter-swift 0.7: struct and enum are also `class_declaration` nodes.
// The scope tree only needs the node kind (not the Swift keyword), so a single
// entry for `class_declaration` covers class/struct/enum.
static SWIFT_SCOPE_KINDS: &[ScopeKind] = &[
    ScopeKind { node_kind: "class_declaration",    name_field: "name" },
    // Kept for grammar versions that use separate node kinds.
    ScopeKind { node_kind: "struct_declaration",   name_field: "name" },
    ScopeKind { node_kind: "enum_declaration",     name_field: "name" },
    ScopeKind { node_kind: "protocol_declaration", name_field: "name" },
    ScopeKind { node_kind: "function_declaration", name_field: "name" },
];

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Extract all symbols and references from Swift source.
pub fn extract(source: &str) -> super::ExtractionResult {
    let lang: tree_sitter::Language = tree_sitter_swift::LANGUAGE.into();

    let mut parser = Parser::new();
    parser
        .set_language(&lang)
        .expect("Failed to load Swift grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return super::ExtractionResult::new(vec![], vec![], true),
    };

    let root = tree.root_node();
    let src = source.as_bytes();
    let has_errors = root.has_error();

    let scope_tree = scope_tree::build(root, src, SWIFT_SCOPE_KINDS);

    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    extract_node(root, src, &scope_tree, &mut symbols, &mut refs, None);

    super::ExtractionResult::new(symbols, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Recursive node visitor
// ---------------------------------------------------------------------------

fn extract_node<'a>(
    node: Node<'a>,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_declaration" => {
                push_import(&child, src, symbols.len(), refs);
            }

            // tree-sitter-swift 0.7: class, struct, and enum declarations all use the
            // node kind `class_declaration`.  The first keyword child (`class`, `struct`,
            // `enum`) distinguishes which kind it is.  Protocol declarations use
            // `protocol_declaration` as before.
            "class_declaration" => {
                let swift_kind = swift_type_decl_kind(&child, src);
                let is_enum = swift_kind == SymbolKind::Enum;
                let all_implements = swift_kind != SymbolKind::Class;
                let idx = push_type_decl(&child, src, scope_tree, swift_kind, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_type_inheritance(&child, src, sym_idx, refs, all_implements);
                }
                if is_enum {
                    recurse_enum_body(&child, src, scope_tree, symbols, refs, idx);
                } else {
                    recurse_into_body(&child, src, scope_tree, symbols, refs, idx);
                }
            }

            // Kept for grammar versions that do use separate node kinds.
            "struct_declaration" => {
                let idx = push_type_decl(&child, src, scope_tree, SymbolKind::Struct, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_type_inheritance(&child, src, sym_idx, refs, true);
                }
                recurse_into_body(&child, src, scope_tree, symbols, refs, idx);
            }

            "enum_declaration" => {
                let idx = push_type_decl(&child, src, scope_tree, SymbolKind::Enum, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_type_inheritance(&child, src, sym_idx, refs, true);
                }
                recurse_enum_body(&child, src, scope_tree, symbols, refs, idx);
            }

            "protocol_declaration" => {
                let idx = push_type_decl(&child, src, scope_tree, SymbolKind::Interface, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_type_inheritance(&child, src, sym_idx, refs, true);
                }
                recurse_into_body(&child, src, scope_tree, symbols, refs, idx);
            }

            "extension_declaration" => {
                let idx = push_extension(&child, src, scope_tree, symbols, parent_index);
                recurse_into_body(&child, src, scope_tree, symbols, refs, idx);
            }

            "function_declaration" => {
                let idx = push_function_decl(&child, src, scope_tree, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_calls_from_body(&body, src, sym_idx, refs);
                    } else if let Some(body) = find_child_by_kind(&child, "code_block") {
                        extract_calls_from_body(&body, src, sym_idx, refs);
                    }
                }
            }

            "initializer_declaration" => {
                let idx = push_init(&child, src, scope_tree, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    if let Some(body) = find_child_by_kind(&child, "code_block") {
                        extract_calls_from_body(&body, src, sym_idx, refs);
                    }
                }
            }

            "deinit_declaration" => {
                push_deinit(&child, src, scope_tree, symbols, parent_index);
            }

            "property_declaration" | "stored_property" | "variable_declaration" => {
                push_property(&child, src, scope_tree, symbols, parent_index);
            }

            "ERROR" | "MISSING" => {}

            _ => {
                extract_node(child, src, scope_tree, symbols, refs, parent_index);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Body recursors
// ---------------------------------------------------------------------------

fn recurse_into_body(
    type_node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    // Possible body field names across grammar versions.
    let body = type_node
        .child_by_field_name("body")
        .or_else(|| find_child_by_kind(type_node, "class_body"))
        .or_else(|| find_child_by_kind(type_node, "struct_body"))
        .or_else(|| find_child_by_kind(type_node, "protocol_body"))
        .or_else(|| find_child_by_kind(type_node, "extension_body"))
        .or_else(|| find_child_by_kind(type_node, "{"));
    if let Some(b) = body {
        extract_node(b, src, scope_tree, symbols, refs, parent_index);
    } else {
        // Body might be mixed with other children; recurse over all.
        let mut cursor = type_node.walk();
        for child in type_node.children(&mut cursor) {
            match child.kind() {
                "class_body" | "struct_body" | "protocol_body" | "extension_body"
                | "enum_body" => {
                    extract_node(child, src, scope_tree, symbols, refs, parent_index);
                }
                _ => {}
            }
        }
    }
}

fn recurse_enum_body(
    enum_node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let enum_qname = parent_index
        .and_then(|i| symbols.get(i))
        .map(|s| s.qualified_name.clone())
        .unwrap_or_default();

    let mut outer = enum_node.walk();
    for child in enum_node.children(&mut outer) {
        // tree-sitter-swift 0.7 uses `enum_class_body`; older versions use `enum_body`.
        if child.kind() != "enum_body" && child.kind() != "enum_class_body" {
            continue;
        }
        let mut cursor = child.walk();
        for item in child.children(&mut cursor) {
            match item.kind() {
                // Older grammar: enum_case_declaration wrapping enum_case_name children.
                "enum_case_declaration" => {
                    let mut ic = item.walk();
                    for case_item in item.children(&mut ic) {
                        if case_item.kind() == "enum_case_name"
                            || case_item.kind() == "enum_entry"
                        {
                            let name_node = case_item
                                .child_by_field_name("name")
                                .or_else(|| find_child_by_kind(&case_item, "simple_identifier"));
                            if let Some(nn) = name_node {
                                let name = node_text(nn, src);
                                push_enum_member(name, &enum_qname, &case_item, scope_tree, symbols, parent_index, src);
                            }
                        }
                    }
                }
                // tree-sitter-swift 0.7: `enum_entry` contains `case` keyword +
                // one or more `simple_identifier` children (comma-separated cases).
                "enum_entry" => {
                    let mut ec = item.walk();
                    for id_node in item.children(&mut ec) {
                        if id_node.kind() == "simple_identifier" {
                            let name = node_text(id_node, src);
                            push_enum_member(name, &enum_qname, &id_node, scope_tree, symbols, parent_index, src);
                        }
                    }
                }
                _ => {
                    // Other declarations inside enum bodies (methods, etc.).
                    extract_node(item, src, scope_tree, symbols, refs, parent_index);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Symbol pushers
// ---------------------------------------------------------------------------

fn push_type_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    kind: SymbolKind,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, src);

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let kw = match kind {
        SymbolKind::Class     => "class",
        SymbolKind::Struct    => "struct",
        SymbolKind::Enum      => "enum",
        SymbolKind::Interface => "protocol",
        _                     => "class",
    };

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("{kw} {name}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

/// Push an `extension_declaration`.
///
/// In tree-sitter-swift, the extended type is found via the `extended_type` field
/// or as the first `user_type` / `type_identifier` child (grammar version dependent).
fn push_extension(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    // Try named field first; fall back to scanning children.
    let name = node
        .child_by_field_name("extended_type")
        .or_else(|| find_child_by_kind(node, "user_type"))
        .or_else(|| find_child_by_kind(node, "type_identifier"))
        .map(|n| node_text(n, src))?;

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Namespace,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("extension {name}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

fn push_function_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, src);

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let kind = if scope.is_some() { SymbolKind::Method } else { SymbolKind::Function };

    let params = node
        .child_by_field_name("params")
        .or_else(|| find_child_by_kind(node, "parameter_clause"))
        .map(|p| node_text(p, src))
        .unwrap_or_default();
    let ret = node
        .child_by_field_name("return_type")
        .or_else(|| find_child_by_kind(node, "function_return_type"))
        .map(|r| format!(" -> {}", node_text(r, src)))
        .unwrap_or_default();
    let signature = Some(format!("func {name}{params}{ret}"));

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name,
        qualified_name,
        kind,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature,
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

fn push_init(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let class_name = scope.map(|s| s.name.as_str()).unwrap_or("init").to_string();
    let qualified_name = scope_tree::qualify(&class_name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let params = find_child_by_kind(node, "parameter_clause")
        .map(|p| node_text(p, src))
        .unwrap_or_default();

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: class_name.clone(),
        qualified_name,
        kind: SymbolKind::Constructor,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("init{params}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

fn push_deinit(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let class_name = scope.map(|s| s.name.as_str()).unwrap_or("deinit").to_string();
    let name = format!("~{class_name}");
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    symbols.push(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::Method,
        visibility: None,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some("deinit".to_string()),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
}

fn push_property(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    // Property name: try `name` field, or first `simple_identifier` in a `pattern`.
    let name_opt = node
        .child_by_field_name("name")
        .map(|n| node_text(n, src))
        .or_else(|| {
            // Scan children for a pattern containing a simple_identifier.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "simple_identifier" {
                    return Some(node_text(child, src));
                }
                if child.kind() == "pattern" {
                    let mut pc = child.walk();
                    for inner in child.children(&mut pc) {
                        if inner.kind() == "simple_identifier" {
                            return Some(node_text(inner, src));
                        }
                    }
                }
            }
            None
        });

    let name = match name_opt {
        Some(n) => n,
        None    => return,
    };

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let text = node_text(*node, src);
    let kw = if text.trim_start().starts_with("let") { "let" } else { "var" };
    let ty = node
        .child_by_field_name("type")
        .or_else(|| find_child_by_kind(node, "type_annotation"))
        .map(|t| format!(": {}", node_text(t, src)))
        .unwrap_or_default();

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Property,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("{kw} {name}{ty}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
}

// ---------------------------------------------------------------------------
// Import extraction
// ---------------------------------------------------------------------------

fn push_import(
    node: &Node,
    src: &[u8],
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // import_declaration children: `import`, import_kind?, import_path_component+
    // Collect all `import_path_component` texts and join with `.`.
    let mut parts: Vec<String> = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_path_component" => {
                parts.push(node_text(child, src));
            }
            // Some grammar versions use identifier directly.
            "identifier" => {
                parts.push(node_text(child, src));
            }
            _ => {}
        }
    }

    if parts.is_empty() {
        return;
    }

    let full = parts.join(".");
    let target = parts.last().cloned().unwrap_or_else(|| full.clone());
    refs.push(ExtractedRef {
        source_symbol_index: current_symbol_count,
        target_name: target,
        kind: EdgeKind::Imports,
        line: node.start_position().row as u32,
        module: Some(full),
    });
}

// ---------------------------------------------------------------------------
// Protocol conformance / inheritance
// ---------------------------------------------------------------------------

/// Extract protocol conformance / inheritance refs.
///
/// tree-sitter-swift 0.7: `inheritance_specifier` nodes appear as direct children
/// of the `class_declaration` node (no `type_inheritance_clause` wrapper).
/// Older grammar versions may still use `type_inheritance_clause`.
///
/// `all_implements`: when true (struct/enum/protocol), all refs are Implements.
/// When false (class), the first is Inherits, the rest are Implements.
fn extract_type_inheritance(
    node: &Node,
    src: &[u8],
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
    all_implements: bool,
) {
    let mut cursor = node.walk();
    let mut first = true;

    for child in node.children(&mut cursor) {
        match child.kind() {
            "type_inheritance_clause" => {
                // Older grammar: iterate inherited_type / inheritance_specifier inside clause.
                let mut ic = child.walk();
                for inherited in child.children(&mut ic) {
                    match inherited.kind() {
                        "inheritance_specifier" | "inherited_type" => {
                            if let Some(name) = inherited_type_name(&inherited, src) {
                                let kind = if all_implements || !first {
                                    EdgeKind::Implements
                                } else {
                                    EdgeKind::Inherits
                                };
                                first = false;
                                refs.push(ExtractedRef {
                                    source_symbol_index: source_idx,
                                    target_name: name,
                                    kind,
                                    line: inherited.start_position().row as u32,
                                    module: None,
                                });
                            }
                        }
                        _ => {}
                    }
                }
            }
            // tree-sitter-swift 0.7: inheritance_specifier is a direct child.
            "inheritance_specifier" | "inherited_type" => {
                if let Some(name) = inherited_type_name(&child, src) {
                    let kind = if all_implements || !first {
                        EdgeKind::Implements
                    } else {
                        EdgeKind::Inherits
                    };
                    first = false;
                    refs.push(ExtractedRef {
                        source_symbol_index: source_idx,
                        target_name: name,
                        kind,
                        line: child.start_position().row as u32,
                        module: None,
                    });
                }
            }
            _ => {}
        }
    }
}

fn inherited_type_name(node: &Node, src: &[u8]) -> Option<String> {
    // inherited_type → user_type → simple_user_type → type_identifier / simple_identifier
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "user_type" => {
                // Take the last simple_user_type's identifier.
                let mut last: Option<String> = None;
                let mut uc = child.walk();
                for ut in child.children(&mut uc) {
                    if ut.kind() == "simple_user_type" {
                        if let Some(id) = ut.child_by_field_name("name")
                            .or_else(|| find_child_by_kind(&ut, "simple_identifier"))
                            .or_else(|| find_child_by_kind(&ut, "type_identifier"))
                        {
                            last = Some(node_text(id, src));
                        }
                    }
                }
                if last.is_some() { return last; }
                // Fallback: full user_type text.
                return Some(node_text(child, src));
            }
            "type_identifier" | "simple_identifier" => {
                return Some(node_text(child, src));
            }
            _ => {}
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Call extraction
// ---------------------------------------------------------------------------

fn extract_calls_from_body(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "call_expression" => {
                // The called value is the first named child.
                if let Some(callee) = child.named_child(0) {
                    let name = call_target_name(&callee, src);
                    if !name.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name: name,
                            kind: EdgeKind::Calls,
                            line: callee.start_position().row as u32,
                            module: None,
                        });
                    }
                }
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            _ => {
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn enclosing_scope<'a>(
    tree: &'a scope_tree::ScopeTree,
    start: usize,
    end: usize,
) -> Option<&'a scope_tree::ScopeEntry> {
    scope_tree::find_enclosing_scope(tree, start, end)
}

fn call_target_name(node: &Node, src: &[u8]) -> String {
    match node.kind() {
        "simple_identifier" | "identifier" | "type_identifier" => node_text(*node, src),
        "navigation_expression" => {
            // member access: last `simple_identifier` in suffix.
            let mut cursor = node.walk();
            let mut last = String::new();
            for child in node.children(&mut cursor) {
                if child.kind() == "navigation_suffix" {
                    let mut nc = child.walk();
                    for inner in child.children(&mut nc) {
                        if inner.kind() == "simple_identifier" {
                            last = node_text(inner, src);
                        }
                    }
                }
            }
            last
        }
        _ => String::new(),
    }
}

/// Push a single EnumMember symbol.
fn push_enum_member(
    name: String,
    enum_qname: &str,
    node: &Node,
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    src: &[u8],
) {
    let qualified_name = if enum_qname.is_empty() {
        name.clone()
    } else {
        format!("{enum_qname}.{name}")
    };
    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    symbols.push(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::EnumMember,
        visibility: None,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: None,
        doc_comment: None,
        scope_path: scope_tree::scope_path(scope),
        parent_index,
    });
}

/// Determine the Swift symbol kind from a `class_declaration` node.
///
/// tree-sitter-swift 0.7 uses `class_declaration` for class, struct, enum, and actor.
/// The `declaration_kind` field carries the keyword token (`class`, `struct`, `enum`, etc.).
fn swift_type_decl_kind(node: &Node, src: &[u8]) -> SymbolKind {
    // Prefer the named `declaration_kind` field.
    if let Some(kw_node) = node.child_by_field_name("declaration_kind") {
        return match kw_node.kind() {
            "struct"     => SymbolKind::Struct,
            "enum"       => SymbolKind::Enum,
            "extension"  => SymbolKind::Namespace,
            "actor"      => SymbolKind::Class,
            _            => SymbolKind::Class, // "class"
        };
    }
    // Fallback: scan direct children for keyword tokens.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "struct"    => return SymbolKind::Struct,
            "enum"      => return SymbolKind::Enum,
            "class"     => return SymbolKind::Class,
            "actor"     => return SymbolKind::Class,
            "extension" => return SymbolKind::Namespace,
            _           => {}
        }
    }
    SymbolKind::Class
}

fn find_child_by_kind<'a>(node: &'a Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    let found = node.children(&mut cursor).find(|c| c.kind() == kind);
    found
}

fn detect_visibility(node: &Node, src: &[u8]) -> Option<Visibility> {
    // Swift modifiers appear as direct children or inside `modifier` nodes.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "modifier" => {
                let text = node_text(child, src);
                match text.trim() {
                    "public"             => return Some(Visibility::Public),
                    "private"            => return Some(Visibility::Private),
                    "fileprivate"        => return Some(Visibility::Private),
                    "internal"           => return Some(Visibility::Internal),
                    _                    => {}
                }
            }
            "visibility_modifier" | "access_level_modifier" => {
                let text = node_text(child, src);
                if text.contains("public")       { return Some(Visibility::Public);    }
                if text.contains("private")      { return Some(Visibility::Private);   }
                if text.contains("fileprivate")  { return Some(Visibility::Private);   }
                if text.contains("internal")     { return Some(Visibility::Internal);  }
            }
            _ => {}
        }
    }
    None
}

fn extract_doc_comment(node: &Node, src: &[u8]) -> Option<String> {
    let mut sib = node.prev_sibling();
    while let Some(s) = sib {
        let text = node_text(s, src);
        let trimmed = text.trim_start();
        if trimmed.starts_with("/**") || trimmed.starts_with("///") {
            return Some(text);
        }
        if trimmed.starts_with("/*") || trimmed.starts_with("//") || trimmed.is_empty() {
            sib = s.prev_sibling();
            continue;
        }
        break;
    }
    None
}

fn node_text(node: Node, src: &[u8]) -> String {
    std::str::from_utf8(&src[node.start_byte()..node.end_byte()])
        .unwrap_or("")
        .to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "swift_tests.rs"]
mod tests;
