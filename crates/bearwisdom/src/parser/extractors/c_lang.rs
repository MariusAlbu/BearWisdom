// =============================================================================
// parser/extractors/c_lang.rs  —  C and C++ symbol and reference extractor
//
// What we extract
// ---------------
// C:
//   Function        — top-level function definitions
//   Struct          — `struct Foo { ... }`
//   Enum            — `enum Foo { ... }`
//   EnumMember      — enum constants
//   TypeAlias       — `typedef` (struct typedefs, function pointer typedefs, plain aliases)
//   Variable        — global variable declarations
//   Import (ref)    — `#include <…>` / `#include "…"` → EdgeKind::Imports
//
// C++ (all of the above, plus):
//   Namespace       — `namespace Foo { … }`
//   Class           — `class Foo { … }` / `struct Foo { … }` (treated as Class in C++)
//   Method          — member function definitions inside a class/struct
//   Constructor     — constructor definition
//   Destructor      — destructor definition (SymbolKind::Method, name = "~ClassName")
//   Inherits (ref)  — `: public Base` / `: Base` → EdgeKind::Inherits
//   Implements (ref)— `: public Interface` (heuristic: same as Inherits at parse time)
//
// Both:
//   Calls (ref)     — `call_expression` → EdgeKind::Calls
//   TypeRef (ref)   — `->` / `.` field access target → EdgeKind::TypeRef (best-effort)
//
// Scope / qualified names
// -----------------------
// The scope_tree is built over namespace_definition and class_specifier nodes.
// Functions inside a class body get the class name prepended (C++ method dispatch).
//
// Grammar notes (tree-sitter-c 0.24 / tree-sitter-cpp 0.23):
//   C:
//     function_definition      → declarator field = declarator (which wraps identifier)
//     struct_specifier         → name field = type_identifier
//     enum_specifier           → name field = type_identifier
//     enumerator               → name field = identifier
//     type_definition          → declarator child gives the aliased name
//     declaration              → declarator field (variable_declarator → identifier)
//     preproc_include          → path child = string_literal | system_lib_string
//   C++:
//     namespace_definition     → name field = namespace_identifier | identifier
//     class_specifier          → name field = type_identifier
//     field_declaration        → declarator field (for member variables)
//     function_definition inside class → same as C function_definition
//     constructor in C++ grammar:  node kind "function_definition" where the
//       declarator is a "function_declarator" whose declarator child is a
//       "qualified_identifier" or "identifier" matching the enclosing class name.
//     base_class_clause        → named child list: base_class_specifier*
//       base_class_specifier   → unnamed children: access_specifier? type_identifier
// =============================================================================

use crate::parser::scope_tree::{self, ScopeKind};
use crate::types::{
    ChainSegment, EdgeKind, ExtractedRef, ExtractedSymbol, MemberChain, SegmentKind, SymbolKind,
    Visibility,
};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Scope configuration
// ---------------------------------------------------------------------------

/// C scope: only struct/enum/union open named scopes (no namespace in C).
static C_SCOPE_KINDS: &[ScopeKind] = &[
    ScopeKind { node_kind: "struct_specifier", name_field: "name" },
    ScopeKind { node_kind: "enum_specifier",   name_field: "name" },
    ScopeKind { node_kind: "union_specifier",  name_field: "name" },
];

/// C++ scope: namespaces + class/struct/union all open scopes.
static CPP_SCOPE_KINDS: &[ScopeKind] = &[
    ScopeKind { node_kind: "namespace_definition", name_field: "name" },
    ScopeKind { node_kind: "class_specifier",      name_field: "name" },
    ScopeKind { node_kind: "struct_specifier",     name_field: "name" },
    ScopeKind { node_kind: "enum_specifier",       name_field: "name" },
    ScopeKind { node_kind: "union_specifier",      name_field: "name" },
];

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Extract all symbols and references from C or C++ source.
///
/// `language` should be `"c"` or `"cpp"`.
pub fn extract(source: &str, language: &str) -> super::ExtractionResult {
    let lang: tree_sitter::Language = if language == "c" {
        tree_sitter_c::LANGUAGE.into()
    } else {
        tree_sitter_cpp::LANGUAGE.into()
    };

    let mut parser = Parser::new();
    parser
        .set_language(&lang)
        .expect("Failed to load C/C++ grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return super::ExtractionResult::new(vec![], vec![], true),
    };

    let root = tree.root_node();
    let src = source.as_bytes();
    let has_errors = root.has_error();

    let scope_config = if language == "c" { C_SCOPE_KINDS } else { CPP_SCOPE_KINDS };
    let scope_tree = scope_tree::build(root, src, scope_config);

    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    extract_node(root, src, &scope_tree, language, &mut symbols, &mut refs, None);

    super::ExtractionResult::new(symbols, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Recursive node visitor
// ---------------------------------------------------------------------------

fn extract_node<'a>(
    node: Node<'a>,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    language: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "preproc_include" => {
                push_include(&child, src, symbols.len(), refs);
            }

            "function_definition" => {
                let idx = push_function_def(&child, src, scope_tree, language, symbols, parent_index);
                // Extract calls from the function body.
                if let Some(sym_idx) = idx {
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_calls_from_body(&body, src, sym_idx, refs);
                    }
                }
            }

            // C: typedef struct / typedef enum / typedef plain alias
            "type_definition" => {
                push_typedef(&child, src, scope_tree, symbols, parent_index);
            }

            // C/C++: loose struct/enum/union specifiers (declarations, not inside class)
            "struct_specifier" | "union_specifier" => {
                let kind = SymbolKind::Struct;
                let idx = push_specifier(&child, src, scope_tree, kind, language, symbols, parent_index);
                // Extract base classes for C++ structs.
                if language != "c" {
                    if let Some(sym_idx) = idx {
                        extract_bases(&child, src, sym_idx, refs);
                    }
                }
                // Recurse into body to get members/methods.
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node(body, src, scope_tree, language, symbols, refs, idx);
                }
            }

            "enum_specifier" => {
                let idx = push_specifier(&child, src, scope_tree, SymbolKind::Enum, language, symbols, parent_index);
                if let Some(body) = child.child_by_field_name("body") {
                    extract_enum_body(&body, src, scope_tree, symbols, idx);
                }
            }

            // C++ class declaration
            "class_specifier" if language != "c" => {
                let idx = push_specifier(&child, src, scope_tree, SymbolKind::Class, language, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_bases(&child, src, sym_idx, refs);
                }
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node(body, src, scope_tree, language, symbols, refs, idx);
                }
            }

            // C++ namespace
            "namespace_definition" if language != "c" => {
                let idx = push_namespace(&child, src, scope_tree, symbols, parent_index);
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node(body, src, scope_tree, language, symbols, refs, idx);
                }
            }

            // Top-level variable / field declarations
            "declaration" | "field_declaration" => {
                push_declaration(&child, src, scope_tree, symbols, parent_index);
            }

            "ERROR" | "MISSING" => {}

            _ => {
                extract_node(child, src, scope_tree, language, symbols, refs, parent_index);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Symbol pushers
// ---------------------------------------------------------------------------

/// Emit an ExtractedSymbol for a `function_definition`.
///
/// In C: `function_definition` → declarator = function_declarator → declarator = identifier
/// In C++: same but may have `qualified_identifier` for qualified methods.
fn push_function_def(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    language: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    let decl_node = node.child_by_field_name("declarator")?;
    let (name, is_destructor) = extract_declarator_name(&decl_node, src);
    let name = name?;

    // Determine enclosing scope for qualified name.
    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    // Decide kind: constructor if name matches the enclosing class, destructor by flag.
    let kind = if is_destructor {
        SymbolKind::Method
    } else if language != "c" && is_constructor_name(&name, scope) {
        SymbolKind::Constructor
    } else if scope.is_some() {
        // Inside a class/struct scope → Method
        SymbolKind::Method
    } else {
        SymbolKind::Function
    };

    let visibility = detect_visibility(node, src);
    let ret_type = node
        .child_by_field_name("type")
        .map(|t| node_text(t, src))
        .unwrap_or_default();
    let params = decl_node
        .child_by_field_name("parameters")
        .or_else(|| find_child_by_kind(&decl_node, "parameter_list"))
        .map(|p| node_text(p, src))
        .unwrap_or_default();
    let signature = Some(format!("{ret_type} {name}{params}").trim().to_string());

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
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

/// Push a `struct_specifier`, `class_specifier`, `enum_specifier`, or `union_specifier`.
fn push_specifier(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    kind: SymbolKind,
    _language: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, src);

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let kw = match kind {
        SymbolKind::Class  => "class",
        SymbolKind::Struct => "struct",
        SymbolKind::Enum   => "enum",
        _                  => "struct",
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

/// Push a `namespace_definition` symbol.
fn push_namespace(
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

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Namespace,
        visibility: None,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("namespace {name}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

/// Push `typedef` aliases.
///
/// `type_definition` children (C grammar):
///   `typedef` keyword, `type` (the underlying type — may be struct_specifier),
///   `declarator` (the new name, often a `type_identifier`).
fn push_typedef(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    // The declarator child carries the aliased name.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "type_identifier" | "pointer_declarator" | "function_declarator" => {
                // Walk into pointer_declarator to find the type_identifier.
                let name = first_type_identifier(&child, src);
                if let Some(name) = name {
                    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
                    let qualified_name = scope_tree::qualify(&name, scope);
                    let scope_path = scope_tree::scope_path(scope);

                    symbols.push(ExtractedSymbol {
                        name: name.clone(),
                        qualified_name,
                        kind: SymbolKind::TypeAlias,
                        visibility: None,
                        start_line: node.start_position().row as u32,
                        end_line: node.end_position().row as u32,
                        start_col: node.start_position().column as u32,
                        end_col: node.end_position().column as u32,
                        signature: Some(format!("typedef {name}")),
                        doc_comment: extract_doc_comment(node, src),
                        scope_path,
                        parent_index,
                    });
                }
                return;
            }
            _ => {}
        }
    }
}

/// Push global variable / field declarations.
fn push_declaration(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    let type_str = node
        .child_by_field_name("type")
        .map(|t| node_text(t, src))
        .unwrap_or_default();

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let scope_path = scope_tree::scope_path(scope);

    // `declarator` children may be one or more variable_declarator / identifier nodes.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let name_opt = match child.kind() {
            "identifier" => Some(node_text(child, src)),
            "init_declarator" | "pointer_declarator" => {
                first_type_identifier(&child, src)
            }
            _ => None,
        };
        if let Some(name) = name_opt {
            let qualified_name = scope_tree::qualify(&name, scope);
            symbols.push(ExtractedSymbol {
                name: name.clone(),
                qualified_name,
                kind: SymbolKind::Variable,
                visibility: detect_visibility(node, src),
                start_line: child.start_position().row as u32,
                end_line: child.end_position().row as u32,
                start_col: child.start_position().column as u32,
                end_col: child.end_position().column as u32,
                signature: Some(format!("{type_str} {name}")),
                doc_comment: None,
                scope_path: scope_path.clone(),
                parent_index,
            });
        }
    }
}

/// Push enum constants from an `enumerator_list`.
fn extract_enum_body(
    body: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    let enum_qname = parent_index
        .and_then(|i| symbols.get(i))
        .map(|s| s.qualified_name.clone())
        .unwrap_or_default();

    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() == "enumerator" {
            if let Some(name_node) = child.child_by_field_name("name") {
                let name = node_text(name_node, src);
                let qualified_name = if enum_qname.is_empty() {
                    name.clone()
                } else {
                    format!("{enum_qname}.{name}")
                };
                let scope = enclosing_scope(scope_tree, child.start_byte(), child.end_byte());
                symbols.push(ExtractedSymbol {
                    name,
                    qualified_name,
                    kind: SymbolKind::EnumMember,
                    visibility: None,
                    start_line: child.start_position().row as u32,
                    end_line: child.end_position().row as u32,
                    start_col: child.start_position().column as u32,
                    end_col: child.end_position().column as u32,
                    signature: None,
                    doc_comment: None,
                    scope_path: scope_tree::scope_path(scope),
                    parent_index,
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Import extraction
// ---------------------------------------------------------------------------

/// Emit an Imports ref for a `preproc_include` node.
///
/// `#include <stdio.h>` → `system_lib_string`
/// `#include "myfile.h"` → `string_literal`
fn push_include(
    node: &Node,
    src: &[u8],
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "string_literal" | "system_lib_string" => {
                let raw = node_text(child, src);
                // Strip surrounding `"`, `<`, `>`.
                let path = raw.trim_matches('"').trim_matches('<').trim_matches('>');
                let target_name = path
                    .rsplit('/')
                    .next()
                    .unwrap_or(path)
                    .to_string();
                refs.push(ExtractedRef {
                    source_symbol_index: current_symbol_count,
                    target_name,
                    kind: EdgeKind::Imports,
                    line: node.start_position().row as u32,
                    module: Some(path.to_string()),
                    chain: None,
                });
                return;
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Inheritance extraction (C++ only)
// ---------------------------------------------------------------------------

/// Extract base class refs from `base_class_clause`.
///
/// tree-sitter-cpp 0.23: `base_class_clause` children are `:`, then directly
/// `access_specifier?` and `type_identifier` (no `base_class_specifier` wrapper).
fn extract_bases(
    node: &Node,
    src: &[u8],
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "base_class_clause" {
            let mut bc = child.walk();
            for base in child.children(&mut bc) {
                match base.kind() {
                    "type_identifier" => {
                        // Direct type_identifier child — the base class name.
                        let name = node_text(base, src);
                        refs.push(ExtractedRef {
                            source_symbol_index: source_idx,
                            target_name: name,
                            kind: EdgeKind::Inherits,
                            line: base.start_position().row as u32,
                            module: None,
                            chain: None,
                        });
                    }
                    "base_class_specifier" => {
                        // Older grammar versions that do wrap in base_class_specifier.
                        let mut ic = base.walk();
                        for inner in base.children(&mut ic) {
                            if inner.kind() == "type_identifier" {
                                let name = node_text(inner, src);
                                refs.push(ExtractedRef {
                                    source_symbol_index: source_idx,
                                    target_name: name,
                                    kind: EdgeKind::Inherits,
                                    line: inner.start_position().row as u32,
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
    }
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
                // `function` field is the callee node.
                if let Some(fn_node) = child.child_by_field_name("function") {
                    let chain = build_chain(fn_node, src);

                    let target_name = chain
                        .as_ref()
                        .and_then(|c| c.segments.last())
                        .map(|s| s.name.clone())
                        .unwrap_or_else(|| call_target_name(&fn_node, src));

                    if !target_name.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name,
                            kind: EdgeKind::Calls,
                            line: fn_node.start_position().row as u32,
                            module: None,
                            chain,
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

/// Build a structured member-access chain from a C/C++ call expression's function node.
///
/// Returns `None` for bare single-segment identifiers.
///
/// C/C++ tree-sitter node shapes:
///   `identifier`          — leaf name
///   `field_identifier`    — member name leaf
///   `this`                — C++ `this` keyword (node kind is "this")
///   `field_expression`    — `obj.field` or `obj->field`
///                           fields: `argument` (receiver), `field` (member name)
///   `qualified_identifier`— `Foo::bar` — fields: `scope`, `name`
///   `call_expression`     — nested call `a.b().c()` — walk into `function` child
fn build_chain(node: Node, src: &[u8]) -> Option<MemberChain> {
    match node.kind() {
        "identifier" | "field_identifier" => return None,
        _ => {}
    }
    let mut segments = Vec::new();
    build_chain_inner(node, src, &mut segments)?;
    if segments.len() < 2 {
        return None;
    }
    Some(MemberChain { segments })
}

fn build_chain_inner(node: Node, src: &[u8], segments: &mut Vec<ChainSegment>) -> Option<()> {
    match node.kind() {
        "identifier" | "field_identifier" | "type_identifier" => {
            segments.push(ChainSegment {
                name: node_text(node, src),
                node_kind: node.kind().to_string(),
                kind: SegmentKind::Identifier,
                declared_type: None,
                optional_chaining: false,
            });
            Some(())
        }

        "this" => {
            segments.push(ChainSegment {
                name: "this".to_string(),
                node_kind: "this".to_string(),
                kind: SegmentKind::SelfRef,
                declared_type: None,
                optional_chaining: false,
            });
            Some(())
        }

        "field_expression" => {
            // `argument` field = receiver, `field` field = member name.
            let argument = node.child_by_field_name("argument")?;
            let field = node.child_by_field_name("field")?;
            build_chain_inner(argument, src, segments)?;
            segments.push(ChainSegment {
                name: node_text(field, src),
                node_kind: field.kind().to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                optional_chaining: false,
            });
            Some(())
        }

        "qualified_identifier" => {
            // `Foo::bar` — emit scope as Identifier, name as Property.
            let scope = node.child_by_field_name("scope");
            let name_node = node.child_by_field_name("name")?;
            if let Some(scope_node) = scope {
                build_chain_inner(scope_node, src, segments)?;
            }
            segments.push(ChainSegment {
                name: node_text(name_node, src),
                node_kind: name_node.kind().to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                optional_chaining: false,
            });
            Some(())
        }

        "call_expression" => {
            // Nested call in a chain: `a.b().c()` — walk into `function` child.
            let func = node.child_by_field_name("function")?;
            build_chain_inner(func, src, segments)
        }

        // Unknown — can't build a chain.
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Walk the enclosing scope, excluding the node's own scope entry.
fn enclosing_scope<'a>(
    tree: &'a scope_tree::ScopeTree,
    start: usize,
    end: usize,
) -> Option<&'a scope_tree::ScopeEntry> {
    scope_tree::find_enclosing_scope(tree, start, end)
}

/// Extract the simple function/method name from a declarator chain.
///
/// Returns `(name, is_destructor)`.
///
/// function_declarator → declarator = identifier | destructor_name | qualified_identifier
fn extract_declarator_name(node: &Node, src: &[u8]) -> (Option<String>, bool) {
    match node.kind() {
        "identifier" | "type_identifier" | "field_identifier" => {
            (Some(node_text(*node, src)), false)
        }
        "destructor_name" => {
            // destructor_name children: `~`, identifier
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    let n = node_text(child, src);
                    return (Some(format!("~{n}")), true);
                }
            }
            (None, true)
        }
        "qualified_identifier" => {
            // Take the last `::` segment as the simple name.
            if let Some(name_node) = node.child_by_field_name("name") {
                return extract_declarator_name(&name_node, src);
            }
            (None, false)
        }
        "function_declarator" => {
            if let Some(d) = node.child_by_field_name("declarator") {
                return extract_declarator_name(&d, src);
            }
            (None, false)
        }
        "pointer_declarator" | "reference_declarator" => {
            if let Some(d) = node.child_by_field_name("declarator") {
                return extract_declarator_name(&d, src);
            }
            (None, false)
        }
        _ => (None, false),
    }
}

/// Returns the simple name string from a `call_expression`'s `function` child.
fn call_target_name(node: &Node, src: &[u8]) -> String {
    match node.kind() {
        "identifier" | "field_identifier" => node_text(*node, src),
        "field_expression" => {
            // `field` field is the member name.
            node.child_by_field_name("field")
                .map(|f| node_text(f, src))
                .unwrap_or_default()
        }
        "qualified_identifier" => {
            node.child_by_field_name("name")
                .map(|n| node_text(n, src))
                .unwrap_or_default()
        }
        _ => String::new(),
    }
}

/// Walk a node tree looking for the first `type_identifier` leaf.
fn first_type_identifier(node: &Node, src: &[u8]) -> Option<String> {
    if node.kind() == "type_identifier" || node.kind() == "identifier" {
        return Some(node_text(*node, src));
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(n) = first_type_identifier(&child, src) {
            return Some(n);
        }
    }
    None
}

/// Find a direct child node by kind.
fn find_child_by_kind<'a>(node: &'a Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    let found = node.children(&mut cursor).find(|c| c.kind() == kind);
    found
}

/// Check if a function name matches the enclosing class name (constructor heuristic).
fn is_constructor_name(name: &str, scope: Option<&scope_tree::ScopeEntry>) -> bool {
    scope.map(|s| s.name.as_str() == name).unwrap_or(false)
}

/// Best-effort visibility from storage class specifiers or access specifiers.
fn detect_visibility(node: &Node, src: &[u8]) -> Option<Visibility> {
    // C/C++ access specifiers appear as siblings in a class body.
    // We scan backward for `access_specifier` siblings.
    let mut sib = node.prev_sibling();
    while let Some(s) = sib {
        match s.kind() {
            "access_specifier" => {
                let text = node_text(s, src);
                let text = text.trim_end_matches(':').trim();
                return match text {
                    "public"    => Some(Visibility::Public),
                    "private"   => Some(Visibility::Private),
                    "protected" => Some(Visibility::Protected),
                    _           => None,
                };
            }
            // Stop scanning at the start of the class body.
            "{" => break,
            _ => {}
        }
        sib = s.prev_sibling();
    }
    None
}

/// Extract a `//` or `/* */` doc comment preceding the node.
fn extract_doc_comment(node: &Node, src: &[u8]) -> Option<String> {
    let mut sib = node.prev_sibling();
    while let Some(s) = sib {
        match s.kind() {
            "comment" => {
                let text = node_text(s, src);
                let trimmed = text.trim_start();
                if trimmed.starts_with("/**") || trimmed.starts_with("///") {
                    return Some(text);
                }
                if trimmed.starts_with("/*") || trimmed.starts_with("//") {
                    sib = s.prev_sibling();
                    continue;
                }
                break;
            }
            _ => break,
        }
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
#[path = "c_lang_tests.rs"]
mod tests;
