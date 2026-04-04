// =============================================================================
// languages/fsharp/extract.rs  —  F# symbol and reference extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Namespace  — `namespace`, `named_module`, `module_defn`
//   Function   — `function_or_value_defn` (has parameters)
//   Variable   — `function_or_value_defn` (no parameters / simple binding)
//   Class      — `type_definition` with `anon_type_defn`
//   Struct     — `type_definition` with `record_type_defn`
//   Enum       — `type_definition` with `union_type_defn` or `enum_type_defn`
//   Interface  — `type_definition` with `interface_type_defn`
//   TypeAlias  — `type_definition` with `type_abbrev_defn`
//
// REFERENCES:
//   Imports    — `import_decl` (`open` declarations)
//   Calls      — `application_expression` (function application)
// =============================================================================

use crate::types::{EdgeKind, ExtractionResult, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::{Node, Parser};

pub fn extract(source: &str) -> ExtractionResult {
    let language: tree_sitter::Language = tree_sitter_fsharp::LANGUAGE_FSHARP.into();
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return ExtractionResult::empty();
    }
    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return ExtractionResult::new(vec![], vec![], true),
    };

    let has_errors = tree.root_node().has_error();
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    visit(tree.root_node(), source, &mut symbols, &mut refs, None);

    ExtractionResult::new(symbols, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Core traversal
// ---------------------------------------------------------------------------

fn visit(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "namespace" | "named_module" => {
                extract_namespace(&child, src, symbols, refs, parent_index);
            }
            "module_defn" => {
                extract_module_defn(&child, src, symbols, refs, parent_index);
            }
            "import_decl" => {
                extract_open(&child, src, symbols.len().saturating_sub(1), refs);
            }
            "function_or_value_defn" => {
                extract_let(&child, src, symbols, refs, parent_index);
            }
            "type_definition" => {
                extract_type_def(&child, src, symbols, refs, parent_index);
            }
            _ => {
                visit(child, src, symbols, refs, parent_index);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Namespace / Module
// ---------------------------------------------------------------------------

fn extract_namespace(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let name = node.child_by_field_name("name")
        .map(|n| node_text(&n, src).to_string())
        .unwrap_or_default();

    if name.is_empty() {
        visit(node.clone(), src, symbols, refs, parent_index);
        return;
    }

    let line = node.start_position().row as u32;
    let kw = node.kind();
    let sig = format!("{} {}", kw, name);
    let idx = symbols.len();

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Namespace,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(sig),
        doc_comment: None,
        scope_path: None,
        parent_index,
    });

    visit(*node, src, symbols, refs, Some(idx));
}

fn extract_module_defn(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    // module_defn: `[access] module identifier = ...`
    let name = first_identifier_text(node, src);
    if name.is_empty() {
        visit(*node, src, symbols, refs, parent_index);
        return;
    }

    let line = node.start_position().row as u32;
    let idx = symbols.len();

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Namespace,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(format!("module {}", name)),
        doc_comment: None,
        scope_path: None,
        parent_index,
    });

    visit(*node, src, symbols, refs, Some(idx));
}

// ---------------------------------------------------------------------------
// open declaration → Imports
// ---------------------------------------------------------------------------

fn extract_open(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // import_decl: `open LongIdentifier`
    let text = node_text(node, src);
    let module = text.trim_start_matches("open").trim().to_string();
    if module.is_empty() {
        return;
    }
    refs.push(ExtractedRef {
        source_symbol_index,
        target_name: module.clone(),
        kind: EdgeKind::Imports,
        line: node.start_position().row as u32,
        module: Some(module),
        chain: None,
    });
}

// ---------------------------------------------------------------------------
// let binding → Function / Variable
// ---------------------------------------------------------------------------

fn extract_let(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    // function_or_value_defn: `let [rec] name [params] [: type] = body`
    // Name is in function_declaration_left or value_declaration_left
    let name = extract_let_name(node, src);
    if name.is_empty() {
        return;
    }

    // Determine if it's a function (has parameters) by checking for parameter nodes
    let has_params = has_function_params(node, src);
    let kind = if has_params { SymbolKind::Function } else { SymbolKind::Variable };
    let line = node.start_position().row as u32;
    let idx = symbols.len();

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(format!("let {}", name)),
        doc_comment: None,
        scope_path: None,
        parent_index,
    });

    // Collect calls in the body
    collect_applications(node, src, idx, refs);
}

fn extract_let_name(node: &Node, src: &str) -> String {
    // Walk children looking for the declaration LHS
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_declaration_left" => {
                // Has a direct `identifier` child for the function name
                return first_identifier_text(&child, src);
            }
            "value_declaration_left" => {
                // value_declaration_left → identifier_pattern → long_identifier_or_op
                // The `identifier_pattern` holds the binding name(s).
                // We want the first long_identifier_or_op inside the first
                // identifier_pattern — that is the binding name.
                return extract_value_decl_name(&child, src);
            }
            _ => {}
        }
    }
    // Fallback: first identifier
    first_identifier_text(node, src)
}

/// Extract the binding name from a `value_declaration_left` node.
///
/// The structure is:
///   value_declaration_left
///     identifier_pattern
///       long_identifier_or_op   ← this is the name
///       [identifier_pattern …]  ← these are parameters (ignored here)
fn extract_value_decl_name(node: &Node, src: &str) -> String {
    // First named child should be identifier_pattern
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier_pattern" {
            // First child of identifier_pattern is long_identifier_or_op
            let mut ic = child.walk();
            for ipc in child.children(&mut ic) {
                if ipc.kind() == "long_identifier_or_op" {
                    let t = node_text(&ipc, src).to_string();
                    if !t.is_empty() {
                        return t;
                    }
                }
            }
            // Fallback: direct identifier under identifier_pattern
            return first_identifier_text(&child, src);
        }
    }
    // Fallback: direct identifier under value_declaration_left
    first_identifier_text(node, src)
}

fn has_function_params(node: &Node, src: &str) -> bool {
    let _ = src;
    // If function_declaration_left has more than one identifier child, it has params
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "function_declaration_left" {
            // count identifier/pattern children beyond the first (name)
            let mut c2 = child.walk();
            let count = child.children(&mut c2)
                .filter(|n| n.kind() == "identifier" || n.kind() == "typed_pattern" || n.kind() == "argument_patterns")
                .count();
            return count > 1;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Type definition
// ---------------------------------------------------------------------------

fn extract_type_def(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    // type_definition contains one of: anon_type_defn, record_type_defn,
    // union_type_defn, enum_type_defn, interface_type_defn, type_abbrev_defn
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = match child.kind() {
            "anon_type_defn" => SymbolKind::Class,
            "record_type_defn" => SymbolKind::Struct,
            "union_type_defn" | "enum_type_defn" => SymbolKind::Enum,
            "interface_type_defn" => SymbolKind::Interface,
            "type_abbrev_defn" | "delegate_type_defn" => SymbolKind::TypeAlias,
            _ => continue,
        };

        let name = extract_type_name(&child, src);
        if name.is_empty() {
            continue;
        }

        let line = child.start_position().row as u32;
        let idx = symbols.len();

        symbols.push(ExtractedSymbol {
            name: name.clone(),
            qualified_name: name.clone(),
            kind,
            visibility: Some(Visibility::Public),
            start_line: line,
            end_line: child.end_position().row as u32,
            start_col: child.start_position().column as u32,
            end_col: 0,
            signature: Some(format!("type {}", name)),
            doc_comment: None,
            scope_path: None,
            parent_index,
        });

        // Walk members
        visit(child, src, symbols, refs, Some(idx));
        break; // Only one body per type_definition
    }
}

fn extract_type_name(node: &Node, src: &str) -> String {
    // type_name child → type_name → identifier
    if let Some(tn) = node.child_by_field_name("type_name") {
        if let Some(inner) = tn.child_by_field_name("type_name") {
            return node_text(&inner, src).to_string();
        }
        return first_identifier_text(&tn, src);
    }
    first_identifier_text(node, src)
}

// ---------------------------------------------------------------------------
// Collect application_expression calls
// ---------------------------------------------------------------------------

fn collect_applications(
    node: &Node,
    src: &str,
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "application_expression" {
            // First child is the function being applied
            if let Some(func_child) = child.child(0) {
                match func_child.kind() {
                    "long_identifier_or_op" | "identifier" => {
                        let name = node_text(&func_child, src).to_string();
                        if !name.is_empty() && !is_keyword(&name) {
                            refs.push(ExtractedRef {
                                source_symbol_index: source_idx,
                                target_name: name,
                                kind: EdgeKind::Calls,
                                line: child.start_position().row as u32,
                                module: None,
                                chain: None,
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
        collect_applications(&child, src, source_idx, refs);
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn node_text<'a>(node: &Node, src: &'a str) -> &'a str {
    node.utf8_text(src.as_bytes()).unwrap_or("")
}

fn first_identifier_text(node: &Node, src: &str) -> String {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" {
            let t = node_text(&child, src).to_string();
            if !t.is_empty() {
                return t;
            }
        }
    }
    String::new()
}

fn is_keyword(s: &str) -> bool {
    matches!(s,
        "let" | "in" | "if" | "then" | "else" | "match" | "with"
        | "fun" | "function" | "type" | "and" | "or" | "not"
        | "begin" | "end" | "do" | "done" | "for" | "while"
        | "try" | "finally" | "raise" | "failwith" | "failwithf"
        | "true" | "false" | "null" | "void" | "open" | "module"
        | "namespace" | "of" | "rec" | "mutable" | "new" | "inherit"
        | "override" | "abstract" | "static" | "member" | "val"
        | "interface" | "class" | "struct" | "exception" | "yield"
        | "return" | "async" | "seq" | "task" | "query"
    )
}
