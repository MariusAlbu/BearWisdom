// =============================================================================
// languages/lua/extract.rs  —  Lua symbol and reference extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Function   — function_declaration (global named function)
//   Function   — variable_declaration / assignment with function_definition RHS
//   Method     — function_declaration with dot_index_expression or method_index_expression name
//   Method     — assignment_statement where LHS is dot_index_expression and RHS is function_definition
//   Method     — field inside table_constructor where value is function_definition
//   Class      — assignment_statement or variable_declaration where RHS is table_constructor (heuristic)
//   Field      — field inside table_constructor (non-function value)
//
// REFERENCES:
//   Imports    — function_call where callee identifier = "require" → string arg
//   Calls      — function_call → callee name
//   Calls      — function_call with method_index_expression (colon call)
// =============================================================================

use crate::parser::scope_tree::{self, ScopeKind};
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use crate::types::ExtractionResult;
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Scope configuration
// ---------------------------------------------------------------------------

pub(crate) static LUA_SCOPE_KINDS: &[ScopeKind] = &[
    ScopeKind { node_kind: "function_declaration", name_field: "name" },
];

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn extract(source: &str) -> ExtractionResult {
    let lang: tree_sitter::Language = tree_sitter_lua::LANGUAGE.into();

    let mut parser = Parser::new();
    parser.set_language(&lang).expect("Failed to load Lua grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return ExtractionResult::new(vec![], vec![], true),
    };

    let root = tree.root_node();
    let src = source.as_bytes();
    let has_errors = root.has_error();

    let scope_tree = scope_tree::build(root, src, LUA_SCOPE_KINDS);

    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    visit(root, src, &scope_tree, &mut symbols, &mut refs, None);

    ExtractionResult::new(symbols, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Traversal
// ---------------------------------------------------------------------------

fn visit(
    node: Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_declaration" => {
                let idx = extract_function_declaration(&child, src, scope_tree, symbols, parent_index);
                visit(child, src, scope_tree, symbols, refs, idx.or(parent_index));
            }
            "local_function" => {
                // `local function name(...) ... end`
                let idx = extract_local_function(&child, src, scope_tree, symbols, parent_index);
                visit(child, src, scope_tree, symbols, refs, idx.or(parent_index));
            }
            "variable_declaration" => {
                // Could be: local name = function(...) or local Name = {}
                let idx = extract_variable_declaration(&child, src, scope_tree, symbols, refs, parent_index);
                // Recurse into body for nested functions
                if let Some(body) = child.child_by_field_name("body") {
                    visit(body, src, scope_tree, symbols, refs, idx.or(parent_index));
                } else {
                    visit(child, src, scope_tree, symbols, refs, idx.or(parent_index));
                }
            }
            "assignment_statement" => {
                let idx = extract_assignment_statement(&child, src, scope_tree, symbols, refs, parent_index);
                visit(child, src, scope_tree, symbols, refs, idx.or(parent_index));
            }
            "function_call" => {
                extract_function_call(&child, src, symbols, refs, parent_index);
                visit(child, src, scope_tree, symbols, refs, parent_index);
            }
            "table_constructor" => {
                // Visit table fields directly so every `field` node is always emitted
                extract_all_fields(&child, src, parent_index, symbols, refs);
                visit(child, src, scope_tree, symbols, refs, parent_index);
            }
            _ => {
                visit(child, src, scope_tree, symbols, refs, parent_index);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// function_declaration  →  Function or Method
// ---------------------------------------------------------------------------

fn extract_function_declaration(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    // The name field can be:
    //   identifier                  → global function
    //   dot_index_expression        → Table.method (Method)
    //   method_index_expression     → Table:method (Method)
    let name_node = node.child_by_field_name("name")?;
    let (name, qualified_name, kind) = resolve_func_name(name_node, src);
    if name.is_empty() {
        return None;
    }

    let params = extract_param_list(node, src);
    let signature = format!("function {}({})", qualified_name, params);
    let scope = scope_tree::find_enclosing_scope(scope_tree, node.start_byte(), node.end_byte()).map(|s| s.qualified_name.clone());

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name,
        qualified_name: if let Some(p) = &scope { format!("{}.{}", p, qualified_name) } else { qualified_name },
        kind,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(signature),
        doc_comment: None,
        scope_path: scope,
        parent_index,
    });
    Some(idx)
}

fn extract_local_function(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    // `local function name ...`
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, src);
    if name.is_empty() {
        return None;
    }

    let params = extract_param_list(node, src);
    let signature = format!("local function {}({})", name, params);
    let scope = scope_tree::find_enclosing_scope(scope_tree, node.start_byte(), node.end_byte()).map(|s| s.qualified_name.clone());

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: if let Some(p) = &scope { format!("{}.{}", p, name) } else { name },
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Private),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(signature),
        doc_comment: None,
        scope_path: scope,
        parent_index,
    });
    Some(idx)
}

// ---------------------------------------------------------------------------
// variable_declaration  →  Function or Class
// ---------------------------------------------------------------------------

fn extract_variable_declaration(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) -> Option<usize> {
    // tree-sitter-lua has two forms for variable_declaration:
    //
    // 1. `local name = <rhs>` — inner assignment_statement child:
    //      variable_declaration
    //        (local keyword)
    //        assignment_statement
    //          variable_list → name list
    //          expression_list → value list
    //
    // 2. `local name` (no initializer) — variable_list is a direct child:
    //      variable_declaration
    //        (local keyword)
    //        variable_list
    //          identifier
    //
    // The outer variable_declaration has no named fields; we look at children directly.
    // Keep `inner_opt` alive for the full function scope to satisfy the borrow checker.
    let inner_opt = find_first_named_child_of_kind(node, "assignment_statement");
    let (name_list, rhs) = if let Some(ref inner) = inner_opt {
        // Form 1: `local name = <rhs>` — inner assignment_statement wraps the data
        let nl = find_first_named_child_of_kind(inner, "variable_list")?;
        let value_list = find_first_named_child_of_kind(inner, "expression_list");
        let rhs_node = value_list.and_then(|vl| vl.named_child(0));
        (nl, rhs_node)
    } else if let Some(nl) = find_first_named_child_of_kind(node, "variable_list") {
        // Form 2: `local name` — no initializer; variable_list is a direct child
        (nl, None)
    } else {
        return None;
    };

    let first_name_node = name_list.named_child(0)?;
    let name = node_text(first_name_node, src);
    if name.is_empty() {
        return None;
    }

    let scope = scope_tree::find_enclosing_scope(scope_tree, node.start_byte(), node.end_byte()).map(|s| s.qualified_name.clone());

    let (kind, sig) = if let Some(rhs_node) = rhs {
        match rhs_node.kind() {
            "function_definition" => {
                let params = extract_param_list_from(&rhs_node, src);
                (SymbolKind::Function, Some(format!("local {} = function({})", name, params)))
            }
            "table_constructor" => {
                // Also extract fields from the table (for coverage of `field` nodes)
                let idx = symbols.len();
                symbols.push(ExtractedSymbol {
                    name: name.clone(),
                    qualified_name: if let Some(p) = &scope { format!("{}.{}", p, name) } else { name.clone() },
                    kind: SymbolKind::Class,
                    visibility: Some(Visibility::Private),
                    start_line: node.start_position().row as u32,
                    end_line: node.end_position().row as u32,
                    start_col: node.start_position().column as u32,
                    end_col: node.end_position().column as u32,
                    signature: Some(format!("local {} = {{}}", name)),
                    doc_comment: None,
                    scope_path: scope,
                    parent_index,
                });
                extract_table_fields(&rhs_node, src, idx, symbols, refs);
                return Some(idx);
            }
            "function_call" => {
                extract_function_call(&rhs_node, src, symbols, refs, parent_index);
                (SymbolKind::Variable, None)
            }
            _ => (SymbolKind::Variable, None),
        }
    } else {
        (SymbolKind::Variable, None)
    };

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: if let Some(p) = &scope { format!("{}.{}", p, name) } else { name },
        kind,
        visibility: Some(Visibility::Private),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: sig,
        doc_comment: None,
        scope_path: scope,
        parent_index,
    });
    Some(idx)
}

// ---------------------------------------------------------------------------
// assignment_statement  →  Method, Class, or import via require
// ---------------------------------------------------------------------------

fn extract_assignment_statement(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) -> Option<usize> {
    // Structure: variable_list = expression_list
    // The grammar has lhs as first child (variable_list) and rhs after `=`
    let var_list = find_first_named_child_of_kind(node, "variable_list")
        .or_else(|| find_first_named_child_of_kind(node, "variable"))?;
    let exp_list = find_first_named_child_of_kind(node, "expression_list");

    let lhs = {
        // If var_list, get first named child; otherwise use var_list itself
        if var_list.kind() == "variable_list" {
            var_list.named_child(0)?
        } else {
            var_list
        }
    };

    let rhs = exp_list.and_then(|el| el.named_child(0));
    let scope = scope_tree::find_enclosing_scope(scope_tree, node.start_byte(), node.end_byte()).map(|s| s.qualified_name.clone());

    match lhs.kind() {
        "dot_index_expression" | "method_index_expression" => {
            let method_name = get_index_field_name(&lhs, src);
            let table_name = get_index_table_name(&lhs, src);
            if method_name.is_empty() {
                return None;
            }
            let qname = if table_name.is_empty() {
                method_name.clone()
            } else {
                format!("{}.{}", table_name, method_name)
            };
            let (kind, sig) = if let Some(rhs_node) = rhs {
                if rhs_node.kind() == "function_definition" {
                    let params = extract_param_list_from(&rhs_node, src);
                    (SymbolKind::Method, Some(format!("function {}({})", qname, params)))
                } else {
                    if rhs_node.kind() == "function_call" {
                        extract_function_call(&rhs_node, src, symbols, refs, parent_index);
                    }
                    (SymbolKind::Field, None)
                }
            } else {
                (SymbolKind::Field, None)
            };
            let idx = symbols.len();
            symbols.push(ExtractedSymbol {
                name: method_name,
                qualified_name: qname,
                kind,
                visibility: Some(Visibility::Public),
                start_line: node.start_position().row as u32,
                end_line: node.end_position().row as u32,
                start_col: node.start_position().column as u32,
                end_col: node.end_position().column as u32,
                signature: sig,
                doc_comment: None,
                scope_path: scope,
                parent_index,
            });
            Some(idx)
        }
        "identifier" => {
            let name = node_text(lhs, src);
            if name.is_empty() {
                return None;
            }
            let (kind, sig) = if let Some(rhs_node) = rhs {
                match rhs_node.kind() {
                    "function_definition" => {
                        let params = extract_param_list_from(&rhs_node, src);
                        (SymbolKind::Function, Some(format!("function({})", params)))
                    }
                    "table_constructor" => {
                        let idx = symbols.len();
                        symbols.push(ExtractedSymbol {
                            name: name.clone(),
                            qualified_name: if let Some(p) = &scope { format!("{}.{}", p, name) } else { name.clone() },
                            kind: SymbolKind::Class,
                            visibility: Some(Visibility::Public),
                            start_line: node.start_position().row as u32,
                            end_line: node.end_position().row as u32,
                            start_col: node.start_position().column as u32,
                            end_col: node.end_position().column as u32,
                            signature: Some(format!("{} = {{}}", name)),
                            doc_comment: None,
                            scope_path: scope,
                            parent_index,
                        });
                        extract_table_fields(&rhs_node, src, idx, symbols, refs);
                        return Some(idx);
                    }
                    "function_call" => {
                        extract_function_call(&rhs_node, src, symbols, refs, parent_index);
                        (SymbolKind::Variable, None)
                    }
                    _ => (SymbolKind::Variable, None),
                }
            } else {
                (SymbolKind::Variable, None)
            };
            let idx = symbols.len();
            symbols.push(ExtractedSymbol {
                name: name.clone(),
                qualified_name: if let Some(p) = &scope { format!("{}.{}", p, name) } else { name },
                kind,
                visibility: Some(Visibility::Public),
                start_line: node.start_position().row as u32,
                end_line: node.end_position().row as u32,
                start_col: node.start_position().column as u32,
                end_col: node.end_position().column as u32,
                signature: sig,
                doc_comment: None,
                scope_path: scope,
                parent_index,
            });
            Some(idx)
        }
        _ => {
            // Fallback: use raw LHS text as symbol name (bracket_index_expression, etc.)
            let name = node_text(lhs, src);
            if name.is_empty() {
                return None;
            }
            // Truncate to avoid emitting giant expressions as names
            let short_name = name.split(|c| c == '[' || c == '.' || c == ':').next().unwrap_or(&name).trim().to_string();
            if short_name.is_empty() {
                return None;
            }
            let idx = symbols.len();
            symbols.push(ExtractedSymbol {
                name: short_name.clone(),
                qualified_name: short_name,
                kind: SymbolKind::Variable,
                visibility: Some(Visibility::Public),
                start_line: node.start_position().row as u32,
                end_line: node.end_position().row as u32,
                start_col: node.start_position().column as u32,
                end_col: node.end_position().column as u32,
                signature: None,
                doc_comment: None,
                scope_path: scope,
                parent_index,
            });
            Some(idx)
        }
    }
}

// ---------------------------------------------------------------------------
// Table fields
// ---------------------------------------------------------------------------

fn extract_table_fields(
    table_node: &Node,
    src: &[u8],
    parent_idx: usize,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut field_idx: usize = 0;
    let mut cursor = table_node.walk();
    for field in table_node.children(&mut cursor) {
        if field.kind() != "field" {
            continue;
        }
        let name = if let Some(n) = field.child_by_field_name("name") {
            let t = node_text(n, src);
            if t.is_empty() { format!("_{}", field_idx) } else { t }
        } else {
            format!("_{}", field_idx)
        };
        field_idx += 1;

        let value_node = field.child_by_field_name("value");
        let kind = if value_node.map_or(false, |v| v.kind() == "function_definition") {
            SymbolKind::Method
        } else {
            SymbolKind::Field
        };
        let _ = refs;
        symbols.push(ExtractedSymbol {
            name: name.clone(),
            qualified_name: name,
            kind,
            visibility: Some(Visibility::Public),
            start_line: field.start_position().row as u32,
            end_line: field.end_position().row as u32,
            start_col: field.start_position().column as u32,
            end_col: field.end_position().column as u32,
            signature: None,
            doc_comment: None,
            scope_path: None,
            parent_index: Some(parent_idx),
        });
    }
}

// ---------------------------------------------------------------------------
// function_call  →  Calls + Imports (require)
// ---------------------------------------------------------------------------

fn extract_function_call(
    node: &Node,
    src: &[u8],
    symbols: &[ExtractedSymbol],
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let source_idx = parent_index.unwrap_or_else(|| symbols.len().saturating_sub(1));
    let line = node.start_position().row as u32;

    // tree-sitter-lua 0.5 uses the `name` field for the callee of function_call
    let callee = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };

    match callee.kind() {
        "identifier" => {
            let name = node_text(callee, src);
            if name == "require" {
                // Extract the module path
                if let Some(module_path) = extract_require_arg(node, src) {
                    refs.push(ExtractedRef {
                        source_symbol_index: source_idx,
                        target_name: module_path.clone(),
                        kind: EdgeKind::Imports,
                        line,
                        module: Some(module_path),
                        chain: None,
                    });
                }
            } else if !name.is_empty() {
                refs.push(ExtractedRef {
                    source_symbol_index: source_idx,
                    target_name: name,
                    kind: EdgeKind::Calls,
                    line,
                    module: None,
                    chain: None,
                });
            }
        }
        "dot_index_expression" => {
            let method = get_index_field_name(&callee, src);
            if !method.is_empty() {
                refs.push(ExtractedRef {
                    source_symbol_index: source_idx,
                    target_name: method,
                    kind: EdgeKind::Calls,
                    line,
                    module: None,
                    chain: None,
                });
            }
        }
        "method_index_expression" => {
            let method = get_method_name(&callee, src);
            if !method.is_empty() {
                refs.push(ExtractedRef {
                    source_symbol_index: source_idx,
                    target_name: method,
                    kind: EdgeKind::Calls,
                    line,
                    module: None,
                    chain: None,
                });
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn resolve_func_name(name_node: Node, src: &[u8]) -> (String, String, SymbolKind) {
    match name_node.kind() {
        "identifier" => {
            let n = node_text(name_node, src);
            (n.clone(), n, SymbolKind::Function)
        }
        "dot_index_expression" => {
            let field = get_index_field_name(&name_node, src);
            let table = get_index_table_name(&name_node, src);
            let qname = if table.is_empty() { field.clone() } else { format!("{}.{}", table, field) };
            (field, qname, SymbolKind::Method)
        }
        "method_index_expression" => {
            let method = get_method_name(&name_node, src);
            let table = get_method_table(&name_node, src);
            let qname = if table.is_empty() { method.clone() } else { format!("{}:{}", table, method) };
            (method, qname, SymbolKind::Method)
        }
        _ => (String::new(), String::new(), SymbolKind::Function),
    }
}

fn extract_param_list(node: &Node, src: &[u8]) -> String {
    if let Some(params) = node.child_by_field_name("parameters") {
        extract_param_list_from(&params, src)
    } else {
        String::new()
    }
}

fn extract_param_list_from(node: &Node, src: &[u8]) -> String {
    // Walk children looking for parameter list
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "parameters" || child.kind() == "par_list" {
            return collect_param_text(child, src);
        }
    }
    // Node itself might be a function_definition; try its parameters field
    if let Some(params) = node.child_by_field_name("parameters") {
        return collect_param_text(params, src);
    }
    String::new()
}

fn collect_param_text(params_node: Node, src: &[u8]) -> String {
    let mut cursor = params_node.walk();
    let parts: Vec<String> = params_node
        .children(&mut cursor)
        .filter(|c| c.kind() == "identifier" || c.kind() == "vararg_expression")
        .map(|c| node_text(c, src))
        .collect();
    parts.join(", ")
}

fn get_index_field_name(node: &Node, src: &[u8]) -> String {
    node.child_by_field_name("field")
        .map(|n| node_text(n, src))
        .unwrap_or_default()
}

fn get_index_table_name(node: &Node, src: &[u8]) -> String {
    node.child_by_field_name("table")
        .map(|n| node_text(n, src))
        .unwrap_or_default()
}

fn get_method_name(node: &Node, src: &[u8]) -> String {
    node.child_by_field_name("method")
        .map(|n| node_text(n, src))
        .unwrap_or_default()
}

fn get_method_table(node: &Node, src: &[u8]) -> String {
    node.child_by_field_name("value")
        .map(|n| node_text(n, src))
        .unwrap_or_default()
}

fn extract_require_arg(call_node: &Node, src: &[u8]) -> Option<String> {
    // tree-sitter-lua 0.5 uses the `arguments` field for function call args
    let args = call_node.child_by_field_name("arguments")?;
    let mut cursor = args.walk();
    for child in args.children(&mut cursor) {
        if child.kind() == "string" {
            let raw = node_text(child, src);
            // Strip quotes
            let stripped = raw.trim_matches(|c| c == '"' || c == '\'');
            if !stripped.is_empty() {
                return Some(stripped.to_string());
            }
        }
    }
    None
}

fn find_first_named_child_of_kind<'a>(node: &'a Node<'a>, kind: &str) -> Option<Node<'a>> {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == kind {
                return Some(child);
            }
        }
    }
    None
}

fn node_text(node: Node, src: &[u8]) -> String {
    std::str::from_utf8(&src[node.start_byte()..node.end_byte()])
        .unwrap_or("")
        .to_string()
}

/// Emit a Field symbol for every `field` child of a table_constructor node.
/// This is called for ALL table_constructor nodes encountered during traversal,
/// so every `field` CST node produces a matching symbol regardless of whether
/// the enclosing table was itself named.
fn extract_all_fields(
    table_node: &Node,
    src: &[u8],
    parent_index: Option<usize>,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let parent_idx = parent_index.unwrap_or_else(|| symbols.len().saturating_sub(1));
    let mut field_idx: usize = 0;
    let mut cursor = table_node.walk();
    for field in table_node.children(&mut cursor) {
        if field.kind() != "field" {
            continue;
        }
        // Try named field first, then bracket field [key]=val, then positional
        let name = if let Some(n) = field.child_by_field_name("name") {
            let t = node_text(n, src);
            if t.is_empty() { format!("_{}", field_idx) } else { t }
        } else {
            // Positional field or [key] = val — use positional index as name
            format!("_{}", field_idx)
        };
        field_idx += 1;

        let value_node = field.child_by_field_name("value");
        let kind = if value_node.map_or(false, |v| v.kind() == "function_definition") {
            SymbolKind::Method
        } else {
            SymbolKind::Field
        };
        let _ = refs;
        symbols.push(ExtractedSymbol {
            name: name.clone(),
            qualified_name: name,
            kind,
            visibility: Some(Visibility::Public),
            start_line: field.start_position().row as u32,
            end_line: field.end_position().row as u32,
            start_col: field.start_position().column as u32,
            end_col: field.end_position().column as u32,
            signature: None,
            doc_comment: None,
            scope_path: None,
            parent_index: Some(parent_idx),
        });
    }
}
