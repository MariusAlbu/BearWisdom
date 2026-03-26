// =============================================================================
// parser/extractors/php.rs  —  PHP symbol and reference extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Namespace, Class, Interface, Trait, Enum (PHP 8.1),
//   Method, Constructor (`__construct`), Function (standalone),
//   Property (class property declarations), EnumMember
//
// REFERENCES:
//   - `use_declaration`              → Imports edges (use Foo\Bar\Baz)
//   - `use_instead_of_clause` /
//     `use_as_clause`                → Imports edges (trait use)
//   - `extends_clause`               → Inherits edges
//   - `implements_list` / `base_clause` (interface extends) → Implements edges
//   - Method calls (`$obj->method()`,
//     `ClassName::staticMethod()`)   → Calls edges
//   - `object_creation_expression`
//     (`new Foo(...)`)               → Instantiates edges
//
// Approach:
//   Two-phase approach matching the Java extractor:
//   1. Build a scope tree for qualified-name generation.
//   2. Single-pass recursive CST walk threading `qualified_prefix`,
//      `inside_class`, and `namespace_prefix`.
//
// Visibility convention:
//   PHP has explicit public/protected/private keywords.  The extractor reads
//   the `visibility_modifier` / `final_modifier` child of declarations.
//   Defaults to Public if no modifier is present.
//
// Grammar notes (tree-sitter-php 0.23+):
//   - `namespace_definition` exposes a `name` field (namespace_name).
//   - `class_declaration` exposes `name`, optional `base_clause`
//     (single superclass), optional `class_implements` (interface list).
//   - `method_declaration` exposes `name`, optional `visibility_modifier`.
//   - `function_definition` (standalone) exposes `name`.
//   - `property_declaration` children include `visibility_modifier` and
//     one or more `property_element` nodes.
//   - `use_declaration` (at the top of a class body for trait use) vs
//     `namespace_use_declaration` (top-level `use` statement).
// =============================================================================

use crate::parser::scope_tree::{self, ScopeKind};
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Scope configuration
// ---------------------------------------------------------------------------

static PHP_SCOPE_KINDS: &[ScopeKind] = &[
    ScopeKind { node_kind: "namespace_definition", name_field: "name" },
    ScopeKind { node_kind: "class_declaration",    name_field: "name" },
    ScopeKind { node_kind: "interface_declaration", name_field: "name" },
    ScopeKind { node_kind: "trait_declaration",    name_field: "name" },
    ScopeKind { node_kind: "enum_declaration",     name_field: "name" },
    ScopeKind { node_kind: "method_declaration",   name_field: "name" },
    ScopeKind { node_kind: "function_definition",  name_field: "name" },
];

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Extract all symbols and references from PHP source code.
pub fn extract(source: &str) -> super::ExtractionResult {
    let language: tree_sitter::Language = tree_sitter_php::LANGUAGE_PHP.into();
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to load PHP grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return super::ExtractionResult::new(vec![], vec![], true),
    };

    let has_errors = tree.root_node().has_error();
    let src = source.as_bytes();
    let root = tree.root_node();

    let _scope_tree = scope_tree::build(root, src, PHP_SCOPE_KINDS);

    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    extract_from_node(root, src, &mut symbols, &mut refs, None, "", "");

    super::ExtractionResult::new(symbols, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Core traversal
// ---------------------------------------------------------------------------

fn extract_from_node(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    // The current namespace prefix (backslash-separated, e.g. "App\\Models").
    namespace_prefix: &str,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "namespace_definition" => {
                extract_namespace(
                    &child,
                    src,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                );
            }

            "namespace_use_declaration" => {
                extract_use_declaration(&child, src, refs, symbols.len());
            }

            "function_definition" => {
                extract_function(
                    &child,
                    src,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    namespace_prefix,
                    false, // not inside a class
                );
            }

            "class_declaration" => {
                extract_class(
                    &child,
                    src,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    namespace_prefix,
                    SymbolKind::Class,
                );
            }

            "interface_declaration" => {
                extract_class(
                    &child,
                    src,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    namespace_prefix,
                    SymbolKind::Interface,
                );
            }

            "trait_declaration" => {
                extract_class(
                    &child,
                    src,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    namespace_prefix,
                    // Traits are closest to a class in terms of symbol semantics.
                    SymbolKind::Class,
                );
            }

            "enum_declaration" => {
                extract_enum(
                    &child,
                    src,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    namespace_prefix,
                );
            }

            "ERROR" | "MISSING" => {}

            _ => {
                extract_from_node(
                    child,
                    src,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    namespace_prefix,
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Namespace
// ---------------------------------------------------------------------------

fn extract_namespace(
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
    let qualified_name = qualify_ns(&name, qualified_prefix);
    let new_prefix = qualified_name.clone();
    // Use the namespace name as both prefix and namespace_prefix for children.
    let ns_prefix = name.replace('\\', ".");

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Namespace,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("namespace {name}")),
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });

    // The body of a namespace is either a `compound_statement` (block syntax)
    // or the remaining file children (brace-less syntax). tree-sitter-php wraps
    // both in a `namespace_definition` with a `body` field or direct children.
    if let Some(body) = node.child_by_field_name("body") {
        extract_from_node(body, src, symbols, refs, Some(idx), &new_prefix, &ns_prefix);
    }
}

// ---------------------------------------------------------------------------
// Class / Interface / Trait
// ---------------------------------------------------------------------------

fn extract_class(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    namespace_prefix: &str,
    kind: SymbolKind,
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
        kind,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: build_class_signature(node, src, &name, kind),
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });

    // tree-sitter-php 0.24: `base_clause` and `class_interface_clause` are unnamed
    // children (not named fields).  Scan all children by node kind.
    let mut cc = node.walk();
    for child in node.children(&mut cc) {
        match child.kind() {
            "base_clause" => {
                // base_clause → `extends` keyword + `name`/`qualified_name` node
                let mut bc = child.walk();
                for base_child in child.children(&mut bc) {
                    if base_child.kind() == "qualified_name"
                        || base_child.kind() == "name"
                        || base_child.kind() == "identifier"
                    {
                        refs.push(ExtractedRef {
                            source_symbol_index: idx,
                            target_name: node_text(&base_child, src),
                            kind: EdgeKind::Inherits,
                            line: base_child.start_position().row as u32,
                            module: None,
                        });
                    }
                }
            }
            "class_interface_clause" => {
                // class_interface_clause → `implements` keyword + `name`/`qualified_name` nodes
                extract_interface_list(&child, src, refs, idx, EdgeKind::Implements);
            }
            _ => {}
        }
    }

    // (Legacy field-based lookup kept as fallback for older grammar versions)
    if refs.iter().all(|r| r.source_symbol_index != idx || r.kind != EdgeKind::Inherits) {
        if let Some(base) = node.child_by_field_name("base_clause") {
            let mut c = base.walk();
            for bc in base.children(&mut c) {
                if bc.kind() == "qualified_name" || bc.kind() == "name" || bc.kind() == "identifier" {
                    refs.push(ExtractedRef {
                        source_symbol_index: idx,
                        target_name: node_text(&bc, src),
                        kind: EdgeKind::Inherits,
                        line: bc.start_position().row as u32,
                        module: None,
                    });
                }
            }
        }
    }
    if let Some(impls) = node.child_by_field_name("class_implements") {
        extract_interface_list(&impls, src, refs, idx, EdgeKind::Implements);
    }

    // Recurse into body
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "declaration_list" {
            extract_class_body(&child, src, symbols, refs, Some(idx), &new_prefix, namespace_prefix);
        }
    }
}

fn extract_interface_list(
    node: &Node,
    src: &[u8],
    refs: &mut Vec<ExtractedRef>,
    class_idx: usize,
    edge_kind: EdgeKind,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "qualified_name" || child.kind() == "name" || child.kind() == "identifier" {
            refs.push(ExtractedRef {
                source_symbol_index: class_idx,
                target_name: node_text(&child, src),
                kind: edge_kind,
                line: child.start_position().row as u32,
                module: None,
            });
        } else {
            // Recurse in case it's wrapped (e.g. `name_list`)
            extract_interface_list(&child, src, refs, class_idx, edge_kind);
        }
    }
}

// ---------------------------------------------------------------------------
// Class body (methods, properties, use statements)
// ---------------------------------------------------------------------------

fn extract_class_body(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    namespace_prefix: &str,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "method_declaration" => {
                extract_method(&child, src, symbols, refs, parent_index, qualified_prefix);
            }
            "property_declaration" => {
                extract_property_declaration(
                    &child,
                    src,
                    symbols,
                    parent_index,
                    qualified_prefix,
                );
            }
            "use_declaration" => {
                // Trait `use` inside class body
                extract_trait_use(&child, src, refs, symbols.len());
            }
            "const_declaration" => {
                extract_const_declaration(&child, src, symbols, parent_index, qualified_prefix);
            }
            "enum_declaration" => {
                extract_enum(
                    &child,
                    src,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    namespace_prefix,
                );
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Method
// ---------------------------------------------------------------------------

fn extract_method(
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
    let visibility = extract_visibility(node, src);

    let kind = if name == "__construct" {
        SymbolKind::Constructor
    } else {
        SymbolKind::Method
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

    if let Some(body) = node.child_by_field_name("body") {
        extract_calls_from_body(&body, src, idx, refs);
    }
}

// ---------------------------------------------------------------------------
// Standalone function
// ---------------------------------------------------------------------------

fn extract_function(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    _namespace_prefix: &str,
    _inside_class: bool,
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
        kind: SymbolKind::Function,
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

    if let Some(body) = node.child_by_field_name("body") {
        extract_calls_from_body(&body, src, idx, refs);
    }
}

// ---------------------------------------------------------------------------
// Property declaration
// ---------------------------------------------------------------------------

fn extract_property_declaration(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let visibility = extract_visibility(node, src);

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "property_element" {
            // The variable name is in the first child (a `variable_name` node).
            let mut vc = child.walk();
            for var in child.children(&mut vc) {
                if var.kind() == "variable_name" || var.kind() == "$variable_name" {
                    let raw = node_text(&var, src);
                    // Strip leading `$`
                    let name = raw.trim_start_matches('$').to_string();
                    let qualified_name = qualify(&name, qualified_prefix);
                    symbols.push(ExtractedSymbol {
                        name,
                        qualified_name,
                        kind: SymbolKind::Property,
                        visibility,
                        start_line: var.start_position().row as u32,
                        end_line: node.end_position().row as u32,
                        start_col: var.start_position().column as u32,
                        end_col: node.end_position().column as u32,
                        signature: None,
                        doc_comment: None,
                        scope_path: scope_from_prefix(qualified_prefix),
                        parent_index,
                    });
                    break;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Const declaration (inside class body)
// ---------------------------------------------------------------------------

fn extract_const_declaration(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "const_element" {
            if let Some(name_node) = child.child_by_field_name("name") {
                let name = node_text(&name_node, src);
                let qualified_name = qualify(&name, qualified_prefix);
                symbols.push(ExtractedSymbol {
                    name,
                    qualified_name,
                    kind: SymbolKind::Field,
                    visibility: Some(Visibility::Public),
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
}

// ---------------------------------------------------------------------------
// Enum (PHP 8.1)
// ---------------------------------------------------------------------------

fn extract_enum(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    _namespace_prefix: &str,
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

    // Enum implements
    if let Some(impls) = node.child_by_field_name("class_implements") {
        extract_interface_list(&impls, src, refs, idx, EdgeKind::Implements);
    }

    // Enum cases and methods
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "enum_declaration_list" {
            let mut lc = child.walk();
            for item in child.children(&mut lc) {
                match item.kind() {
                    "enum_case" => {
                        if let Some(nm) = item.child_by_field_name("name") {
                            let case_name = node_text(&nm, src);
                            let case_qn = qualify(&case_name, &new_prefix);
                            symbols.push(ExtractedSymbol {
                                name: case_name,
                                qualified_name: case_qn,
                                kind: SymbolKind::EnumMember,
                                visibility: Some(Visibility::Public),
                                start_line: item.start_position().row as u32,
                                end_line: item.end_position().row as u32,
                                start_col: item.start_position().column as u32,
                                end_col: item.end_position().column as u32,
                                signature: None,
                                doc_comment: None,
                                scope_path: Some(new_prefix.clone()),
                                parent_index: Some(idx),
                            });
                        }
                    }
                    "method_declaration" => {
                        extract_method(
                            &item,
                            src,
                            symbols,
                            refs,
                            Some(idx),
                            &new_prefix,
                        );
                    }
                    _ => {}
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Use (namespace import) and trait use
// ---------------------------------------------------------------------------

fn extract_use_declaration(
    node: &Node,
    src: &[u8],
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
) {
    // `use Foo\Bar\Baz` or `use Foo\Bar\{Baz, Qux}`
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "namespace_use_clause" => {
                push_use_ref_for_name(&child, src, refs, current_symbol_count);
            }
            "qualified_name" | "name" => {
                let full = node_text(&child, src);
                push_fq_import(full, child.start_position().row as u32, refs, current_symbol_count);
            }
            _ => {}
        }
    }
}

fn push_use_ref_for_name(
    node: &Node,
    src: &[u8],
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "qualified_name" || child.kind() == "name" {
            let full = node_text(&child, src);
            push_fq_import(full, child.start_position().row as u32, refs, current_symbol_count);
            return;
        }
    }
}

/// Push an Imports edge for a fully-qualified PHP name like `Foo\Bar\Baz`.
fn push_fq_import(
    full: String,
    line: u32,
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
) {
    let parts: Vec<&str> = full.split('\\').collect();
    let target = parts.last().unwrap_or(&full.as_str()).to_string();
    let module = if parts.len() > 1 {
        Some(parts[..parts.len() - 1].join("\\"))
    } else {
        None
    };
    refs.push(ExtractedRef {
        source_symbol_index: current_symbol_count,
        target_name: target,
        kind: EdgeKind::Imports,
        line,
        module,
    });
}

fn extract_trait_use(
    node: &Node,
    src: &[u8],
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "qualified_name" || child.kind() == "name" {
            let full = node_text(&child, src);
            push_fq_import(full, child.start_position().row as u32, refs, current_symbol_count);
        }
    }
}

// ---------------------------------------------------------------------------
// Call extraction from a function/method body
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
            // `$obj->method(...)` or `ClassName::staticMethod(...)`
            "member_call_expression" | "static_call_expression" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let callee = node_text(&name_node, src);
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: callee,
                        kind: EdgeKind::Calls,
                        line: name_node.start_position().row as u32,
                        module: None,
                    });
                }
            }

            // `new Foo(...)`
            // tree-sitter-php 0.24: object_creation_expression children are
            // `new` keyword + `name`/`qualified_name` + `arguments`.
            // There is no `class_type` named field.
            "object_creation_expression" => {
                // Try named field first (may not exist in this grammar version).
                let cls_node_opt = if let Some(n) = child.child_by_field_name("class_type") {
                    Some(n)
                } else {
                    // Fall back to first name/qualified_name child.
                    let mut c = child.walk();
                    let mut found = None;
                    for n in child.children(&mut c) {
                        if n.kind() == "name"
                            || n.kind() == "qualified_name"
                            || n.kind() == "identifier"
                            || n.kind() == "variable_name"
                        {
                            found = Some(n);
                            break;
                        }
                    }
                    found
                };
                if let Some(cls_node) = cls_node_opt {
                    let cls_name = node_text(&cls_node, src);
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: cls_name,
                        kind: EdgeKind::Instantiates,
                        line: cls_node.start_position().row as u32,
                        module: None,
                    });
                }
            }

            // Bare function call `foo(...)` — function_call_expression
            "function_call_expression" => {
                if let Some(fn_node) = child.child_by_field_name("function") {
                    let callee = node_text(&fn_node, src);
                    let simple = callee.rsplit('\\').next().unwrap_or(&callee).to_string();
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: simple,
                        kind: EdgeKind::Calls,
                        line: fn_node.start_position().row as u32,
                        module: None,
                    });
                }
            }

            _ => {}
        }
        extract_calls_from_body(&child, src, source_symbol_index, refs);
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn node_text(node: &Node, src: &[u8]) -> String {
    node.utf8_text(src).unwrap_or("").to_string()
}

/// Dot-separated qualifier (used for qualified names within a namespace).
fn qualify(name: &str, prefix: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}.{name}")
    }
}

/// Backslash-separated qualifier for namespace symbols themselves.
fn qualify_ns(name: &str, prefix: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}.{name}")
    }
}

fn scope_from_prefix(prefix: &str) -> Option<String> {
    if prefix.is_empty() { None } else { Some(prefix.to_string()) }
}

/// Read the visibility modifier of a method or property declaration.
/// Defaults to Public if no modifier is present (interfaces, enum methods, etc.).
fn extract_visibility(node: &Node, src: &[u8]) -> Option<Visibility> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "visibility_modifier" {
            let text = node_text(&child, src);
            return match text.as_str() {
                "public" => Some(Visibility::Public),
                "protected" => Some(Visibility::Protected),
                "private" => Some(Visibility::Private),
                _ => Some(Visibility::Public),
            };
        }
    }
    Some(Visibility::Public)
}

fn build_method_signature(node: &Node, src: &[u8], name: &str) -> Option<String> {
    let params = node
        .child_by_field_name("parameters")
        .map(|p| node_text(&p, src))
        .unwrap_or_default();
    let ret = node
        .child_by_field_name("return_type")
        .map(|r| format!(": {}", node_text(&r, src)))
        .unwrap_or_default();
    Some(format!("function {name}{params}{ret}"))
}

fn build_class_signature(
    node: &Node,
    src: &[u8],
    name: &str,
    kind: SymbolKind,
) -> Option<String> {
    let keyword = match kind {
        SymbolKind::Interface => "interface",
        _ => "class",
    };

    let base = node
        .child_by_field_name("base_clause")
        .map(|b| format!(" extends {}", node_text(&b, src).trim_start_matches("extends ").trim()))
        .unwrap_or_default();

    let impls = node
        .child_by_field_name("class_implements")
        .map(|i| format!(" implements {}", node_text(&i, src).trim_start_matches("implements ").trim()))
        .unwrap_or_default();

    Some(format!("{keyword} {name}{base}{impls}"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "php_tests.rs"]
mod tests;
