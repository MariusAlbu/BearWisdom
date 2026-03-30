// =============================================================================
// parser/extractors/kotlin.rs  —  Kotlin symbol and reference extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Class          — `class Foo`, `data class Foo`, `sealed class Foo`
//   Interface      — `interface Foo`
//   Enum           — `enum class Foo`
//   EnumMember     — enum entries
//   Namespace      — `object Foo` (companion objects get Method treatment via parent)
//   Method         — `fun foo(…)` inside a class/object
//   Function       — `fun foo(…)` at top level / file level
//   Constructor    — `constructor(…)` (secondary constructors; primary is part of class sig)
//   Property       — `val`/`var` declarations
//
// REFERENCES:
//   import_list / import_header  → EdgeKind::Imports
//   Inheritance (`:`)            → EdgeKind::Inherits (first super_type that is a class)
//   Interface conformance (`,`)  → EdgeKind::Implements (remaining super_types)
//   call_expression / postfix_unary_expression (function call suffix)
//                                → EdgeKind::Calls
//   object_creation_expression / constructor call
//                                → EdgeKind::Instantiates
//
// Annotations (`@Annotation`) are noted but not promoted to `Test` kind
// (Kotlin test annotations are framework-specific; best handled by a later pass).
//
// Grammar notes (tree-sitter-kotlin-ng 1.1):
//   class_declaration             → name: simple_identifier, modifiers?, primary_constructor?,
//                                   delegation_specifiers?, class_body?
//   object_declaration            → name: simple_identifier, class_body?
//   interface_declaration         → similar to class_declaration
//   enum_class_body               → child of class_declaration when it is an enum class
//   enum_entry                    → name: simple_identifier
//   function_declaration          → name: simple_identifier, function_value_parameters,
//                                   type? (return), function_body?
//   property_declaration          → name: simple_identifier (or destructuring)
//   import_header                 → identifier child carries the dotted path
//   delegation_specifiers         → delegation_specifier* (super types)
//   delegation_specifier          → children: user_type | explicit_delegation
//   user_type                     → simple_user_type+ (dotted)
//   simple_user_type              → simple_identifier
// =============================================================================

use crate::parser::scope_tree::{self, ScopeKind};
use crate::types::{ChainSegment, EdgeKind, ExtractedRef, ExtractedSymbol, MemberChain, SegmentKind, SymbolKind, Visibility};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Scope configuration
// ---------------------------------------------------------------------------

static KOTLIN_SCOPE_KINDS: &[ScopeKind] = &[
    ScopeKind { node_kind: "class_declaration",     name_field: "name" },
    ScopeKind { node_kind: "object_declaration",    name_field: "name" },
    ScopeKind { node_kind: "interface_declaration", name_field: "name" },
    ScopeKind { node_kind: "function_declaration",  name_field: "name" },
];

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Extract all symbols and references from Kotlin source.
pub fn extract(source: &str) -> super::ExtractionResult {
    let lang: tree_sitter::Language = tree_sitter_kotlin_ng::LANGUAGE.into();

    let mut parser = Parser::new();
    parser
        .set_language(&lang)
        .expect("Failed to load Kotlin grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return super::ExtractionResult::new(vec![], vec![], true),
    };

    let root = tree.root_node();
    let src = source.as_bytes();
    let has_errors = root.has_error();

    let scope_tree = scope_tree::build(root, src, KOTLIN_SCOPE_KINDS);

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
            "import_list" => {
                extract_imports(&child, src, symbols.len(), refs);
            }

            // tree-sitter-kotlin-ng 1.1: top-level imports are direct `import` nodes,
            // not wrapped in an `import_list`.
            "import" => {
                emit_import(&child, src, symbols.len(), refs);
            }

            "class_declaration" => {
                let kind = classify_class(&child, src);
                let idx = push_type_decl(&child, src, scope_tree, kind, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_delegation_specifiers(&child, src, sym_idx, refs);
                    extract_class_body(&child, src, scope_tree, symbols, refs, idx);
                }
            }

            "object_declaration" => {
                let idx = push_type_decl(&child, src, scope_tree, SymbolKind::Class, symbols, parent_index);
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node(body, src, scope_tree, symbols, refs, idx);
                }
            }

            "interface_declaration" => {
                let idx = push_type_decl(&child, src, scope_tree, SymbolKind::Interface, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_delegation_specifiers(&child, src, sym_idx, refs);
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_node(body, src, scope_tree, symbols, refs, idx);
                    }
                }
            }

            "function_declaration" => {
                let idx = push_function_decl(&child, src, scope_tree, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_calls_from_body(&body, src, sym_idx, refs);
                    }
                }
            }

            "property_declaration" => {
                push_property_decl(&child, src, scope_tree, symbols, parent_index);
            }

            "secondary_constructor" => {
                push_secondary_constructor(&child, src, scope_tree, symbols, parent_index);
            }

            "ERROR" | "MISSING" => {}

            _ => {
                extract_node(child, src, scope_tree, symbols, refs, parent_index);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Class body dispatcher
// ---------------------------------------------------------------------------

fn extract_class_body(
    class_node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    // enum class has `enum_class_body`, regular classes have `class_body`.
    let mut cursor = class_node.walk();
    for child in class_node.children(&mut cursor) {
        match child.kind() {
            "class_body" => {
                extract_node(child, src, scope_tree, symbols, refs, parent_index);
            }
            "enum_class_body" => {
                extract_enum_class_body(&child, src, scope_tree, symbols, refs, parent_index);
            }
            _ => {}
        }
    }
}

fn extract_enum_class_body(
    body: &Node,
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

    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        match child.kind() {
            "enum_entry" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = node_text(name_node, src);
                    let qualified_name = if enum_qname.is_empty() {
                        name.clone()
                    } else {
                        format!("{enum_qname}.{name}")
                    };
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
                        scope_path: if enum_qname.is_empty() { None } else { Some(enum_qname.clone()) },
                        parent_index,
                    });
                }
            }
            // Enum body can also contain function declarations.
            _ => {
                extract_node(child, src, scope_tree, symbols, refs, parent_index);
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
        SymbolKind::Interface => "interface",
        SymbolKind::Enum      => "enum class",
        _                     => "class",
    };

    let visibility = detect_visibility(node, src);
    let type_params = node
        .child_by_field_name("type_parameters")
        .map(|tp| node_text(tp, src))
        .unwrap_or_default();

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("{kw} {name}{type_params}")),
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

    // Method vs. top-level function — determined by whether there is a class scope.
    let kind = if scope.map(|s| s.node_kind).unwrap_or("") == "function_declaration" {
        SymbolKind::Method
    } else if scope.is_some() {
        SymbolKind::Method
    } else {
        SymbolKind::Function
    };

    let params = node
        .child_by_field_name("function_value_parameters")
        .or_else(|| find_child_by_kind(node, "value_arguments"))
        .map(|p| node_text(p, src))
        .unwrap_or_default();
    let ret = node
        .child_by_field_name("type")
        .map(|t| format!(": {}", node_text(t, src)))
        .unwrap_or_default();
    let signature = Some(format!("fun {name}{params}{ret}"));

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

fn push_property_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    // property_declaration → name: simple_identifier (for non-destructuring)
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None    => return,
    };
    let name = node_text(name_node, src);

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    // Detect val vs var from the first unnamed child keyword.
    let kw = if node_text(*node, src).trim_start().starts_with("val") { "val" } else { "var" };
    let ty = node
        .child_by_field_name("type")
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

fn push_secondary_constructor(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    // The constructor's name is the enclosing class name.
    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let class_name = scope.map(|s| s.name.as_str()).unwrap_or("constructor").to_string();
    let qualified_name = scope_tree::qualify(&class_name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let params = find_child_by_kind(node, "function_value_parameters")
        .map(|p| node_text(p, src))
        .unwrap_or_default();

    symbols.push(ExtractedSymbol {
        name: class_name.clone(),
        qualified_name,
        kind: SymbolKind::Constructor,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("constructor{params}")),
        doc_comment: None,
        scope_path,
        parent_index,
    });
}

// ---------------------------------------------------------------------------
// Import extraction
// ---------------------------------------------------------------------------

fn extract_imports(
    node: &Node,
    src: &[u8],
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "import_header" {
            emit_import(&child, src, current_symbol_count, refs);
        }
    }
}

fn emit_import(
    node: &Node,
    src: &[u8],
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // tree-sitter-kotlin-ng 1.1: `import` node children are `import` keyword +
    // `qualified_identifier` (a chain of `identifier` nodes).
    // Older grammar: `import_header` with an `identifier` child.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "qualified_identifier" => {
                // qualified_identifier contains multiple identifier children separated by `.`.
                // Collect all identifiers and join them.
                let mut parts: Vec<String> = Vec::new();
                let mut ic = child.walk();
                for id in child.children(&mut ic) {
                    if id.kind() == "identifier" {
                        parts.push(node_text(id, src));
                    }
                }
                if parts.is_empty() {
                    // Fallback: use the whole text.
                    let full = node_text(child, src);
                    let target = full.rsplit('.').next().unwrap_or(&full).to_string();
                    refs.push(ExtractedRef {
                        source_symbol_index: current_symbol_count,
                        target_name: target,
                        kind: EdgeKind::Imports,
                        line: child.start_position().row as u32,
                        module: Some(full),
                        chain: None,
                    });
                } else {
                    let target = parts.last().cloned().unwrap_or_default();
                    let full = parts.join(".");
                    refs.push(ExtractedRef {
                        source_symbol_index: current_symbol_count,
                        target_name: target,
                        kind: EdgeKind::Imports,
                        line: child.start_position().row as u32,
                        module: Some(full),
                        chain: None,
                    });
                }
                return;
            }
            "identifier" => {
                let full = node_text(child, src);
                let target = full.rsplit('.').next().unwrap_or(&full).to_string();
                refs.push(ExtractedRef {
                    source_symbol_index: current_symbol_count,
                    target_name: target,
                    kind: EdgeKind::Imports,
                    line: child.start_position().row as u32,
                    module: Some(full),
                    chain: None,
                });
                return;
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Inheritance / interface delegation
// ---------------------------------------------------------------------------

/// Walk `delegation_specifiers` to emit Inherits/Implements refs.
fn extract_delegation_specifiers(
    node: &Node,
    src: &[u8],
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    let mut first_super = true;
    for child in node.children(&mut cursor) {
        if child.kind() == "delegation_specifiers" {
            let mut dc = child.walk();
            for spec in child.children(&mut dc) {
                match spec.kind() {
                    "delegation_specifier" | "annotated_delegation_specifier" => {
                        if let Some(name) = delegation_spec_name(&spec, src) {
                            let kind = if first_super {
                                first_super = false;
                                EdgeKind::Inherits
                            } else {
                                EdgeKind::Implements
                            };
                            refs.push(ExtractedRef {
                                source_symbol_index: source_idx,
                                target_name: name,
                                kind,
                                line: spec.start_position().row as u32,
                                module: None,
                                chain: None,
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Extract the type name from a `delegation_specifier` node.
///
/// tree-sitter-kotlin-ng 1.1: delegation_specifier wraps either:
///   - `constructor_invocation` → `user_type` → `simple_user_type`+ (class inheritance with args)
///   - `user_type` directly (interface conformance without args)
///   - `explicit_delegation`
fn delegation_spec_name(node: &Node, src: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "constructor_invocation" => {
                // constructor_invocation → user_type, value_arguments
                let mut cc = child.walk();
                for inner in child.children(&mut cc) {
                    if inner.kind() == "user_type" {
                        return last_simple_identifier_in_user_type(&inner, src);
                    }
                }
            }
            "user_type" => {
                // Take the last `simple_user_type`'s `simple_identifier`.
                return last_simple_identifier_in_user_type(&child, src);
            }
            "simple_identifier" | "type_identifier" => {
                return Some(node_text(child, src));
            }
            _ => {}
        }
    }
    None
}

fn last_simple_identifier_in_user_type(node: &Node, src: &[u8]) -> Option<String> {
    let mut last: Option<String> = None;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "simple_user_type" => {
                if let Some(id_node) = child.child_by_field_name("name") {
                    last = Some(node_text(id_node, src));
                } else {
                    // Fallback: first named child.
                    let mut ic = child.walk();
                    for inner in child.children(&mut ic) {
                        if inner.kind() == "simple_identifier" || inner.kind() == "identifier" {
                            last = Some(node_text(inner, src));
                            break;
                        }
                    }
                }
            }
            // tree-sitter-kotlin-ng 1.1: user_type may contain `identifier` directly.
            "identifier" | "simple_identifier" => {
                last = Some(node_text(child, src));
            }
            _ => {}
        }
    }
    last
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
            // call_expression → `calleeExpression`, `valueArguments`
            "call_expression" => {
                // The callee is the first child.
                if let Some(callee) = child.named_child(0) {
                    let chain = build_chain(&callee, src);
                    let target_name = chain
                        .as_ref()
                        .and_then(|c| c.segments.last())
                        .map(|s| s.name.clone())
                        .unwrap_or_else(|| call_target_name(&callee, src));
                    if !target_name.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name,
                            kind: EdgeKind::Calls,
                            line: callee.start_position().row as u32,
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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Determine the `SymbolKind` for a `class_declaration` by inspecting modifiers.
fn classify_class(node: &Node, src: &[u8]) -> SymbolKind {
    // Check for enum/sealed/data modifier tokens.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "modifiers" {
            let text = node_text(child, src);
            if text.contains("enum")   { return SymbolKind::Enum;  }
        }
    }
    // Also check for `enum` as a sibling keyword (some grammar versions).
    let full_text = node_text(*node, src);
    if full_text.trim_start().starts_with("enum") {
        return SymbolKind::Enum;
    }
    SymbolKind::Class
}

fn enclosing_scope<'a>(
    tree: &'a scope_tree::ScopeTree,
    start: usize,
    end: usize,
) -> Option<&'a scope_tree::ScopeEntry> {
    scope_tree::find_enclosing_scope(tree, start, end)
}

fn call_target_name(node: &Node, src: &[u8]) -> String {
    match node.kind() {
        "simple_identifier" | "identifier" => node_text(*node, src),
        "navigation_expression" => {
            // `navigation_expression` → expression `.` navigation_suffix
            // The member name is the last `simple_identifier` in navigation_suffix.
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

fn find_child_by_kind<'a>(node: &'a Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    let found = node.children(&mut cursor).find(|c| c.kind() == kind);
    found
}

/// Infer visibility from modifier keywords.
fn detect_visibility(node: &Node, src: &[u8]) -> Option<Visibility> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "modifiers" {
            let text = node_text(child, src);
            if text.contains("public")    { return Some(Visibility::Public);    }
            if text.contains("private")   { return Some(Visibility::Private);   }
            if text.contains("protected") { return Some(Visibility::Protected); }
            if text.contains("internal")  { return Some(Visibility::Internal);  }
            return None;
        }
    }
    None
}

fn extract_doc_comment(node: &Node, src: &[u8]) -> Option<String> {
    let mut sib = node.prev_sibling();
    while let Some(s) = sib {
        let text = node_text(s, src);
        let trimmed = text.trim_start();
        if trimmed.starts_with("/**") {
            return Some(text);
        }
        if trimmed.starts_with("/*") || trimmed.is_empty() {
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
// Member chain builder
// ---------------------------------------------------------------------------

/// Build a structured member access chain from a Kotlin CST node.
///
/// Kotlin uses `navigation_expression` for member access:
///
/// ```text
/// call_expression
///   navigation_expression        ← callee
///     navigation_expression      ← receiver (chained)
///       this_expression
///       navigation_suffix: simple_identifier "repo"
///     navigation_suffix: simple_identifier "findOne"
///   value_arguments
/// ```
/// produces: `[this, repo, findOne]`
fn build_chain(node: &Node, src: &[u8]) -> Option<MemberChain> {
    let mut segments = Vec::new();
    build_chain_inner(node, src, &mut segments)?;
    if segments.is_empty() {
        return None;
    }
    Some(MemberChain { segments })
}

fn build_chain_inner(node: &Node, src: &[u8], segments: &mut Vec<ChainSegment>) -> Option<()> {
    match node.kind() {
        "simple_identifier" | "identifier" => {
            let name = node_text(*node, src);
            segments.push(ChainSegment {
                name,
                node_kind: "simple_identifier".to_string(),
                kind: SegmentKind::Identifier,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "this_expression" => {
            segments.push(ChainSegment {
                name: "this".to_string(),
                node_kind: "this_expression".to_string(),
                kind: SegmentKind::SelfRef,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "super_expression" => {
            segments.push(ChainSegment {
                name: "super".to_string(),
                node_kind: "super_expression".to_string(),
                kind: SegmentKind::SelfRef,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "navigation_expression" => {
            // navigation_expression: receiver `.` navigation_suffix
            // receiver is first named child; navigation_suffix follows.
            let mut cursor = node.walk();
            let mut children = node.children(&mut cursor);
            let receiver = children.find(|c| c.is_named())?;
            build_chain_inner(&receiver, src, segments)?;
            // Find the navigation_suffix and extract simple_identifier from it.
            let mut cursor2 = node.walk();
            for child in node.children(&mut cursor2) {
                if child.kind() == "navigation_suffix" {
                    let mut nc = child.walk();
                    for inner in child.children(&mut nc) {
                        if inner.kind() == "simple_identifier" {
                            segments.push(ChainSegment {
                                name: node_text(inner, src),
                                node_kind: "navigation_suffix".to_string(),
                                kind: SegmentKind::Property,
                                declared_type: None,
                                type_args: vec![],
                                optional_chaining: false,
                            });
                            break;
                        }
                    }
                }
            }
            Some(())
        }

        "call_expression" => {
            // Chained call: callee is first named child.
            let callee = node.named_child(0)?;
            build_chain_inner(&callee, src, segments)
        }

        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "kotlin_tests.rs"]
mod tests;
