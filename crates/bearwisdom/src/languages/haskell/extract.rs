// =============================================================================
// languages/haskell/extract.rs  —  Haskell extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Function    — `function` node at top level (or in class/instance body)
//   Struct      — `data_type` / `data_family`
//   Struct      — `newtype`
//   Interface   — `class` (type class)
//   Class       — `instance`
//   TypeAlias   — `type_synomym` / `type_family`
//   Namespace   — `module` header
//
// REFERENCES:
//   Imports     — `import` node
//   Calls       — `apply` node (function application)
//   Implements  — `instance` → type class name
//   Implements  — deriving clause in data_type / newtype
// =============================================================================

use crate::parser::scope_tree::{self, ScopeKind};
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use crate::types::ExtractionResult;
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Scope configuration
// ---------------------------------------------------------------------------

pub(crate) static HASKELL_SCOPE_KINDS: &[ScopeKind] = &[
    ScopeKind { node_kind: "function",  name_field: "name" },
    ScopeKind { node_kind: "class",     name_field: "name" },
    ScopeKind { node_kind: "data_type", name_field: "name" },
    ScopeKind { node_kind: "newtype",   name_field: "name" },
];

// Haskell built-in type names — skip TypeRef for these.
const BUILTIN_TYPES: &[&str] = &[
    "Int", "Integer", "Float", "Double", "Bool", "Char", "String",
    "IO", "Maybe", "Either", "List", "Ordering", "Word",
    "Int8", "Int16", "Int32", "Int64",
    "Word8", "Word16", "Word32", "Word64",
    "Natural", "Rational", "Complex",
    "()", "[]",
];

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn extract(source: &str) -> ExtractionResult {
    let lang: tree_sitter::Language = tree_sitter_haskell::LANGUAGE.into();

    let mut parser = Parser::new();
    parser.set_language(&lang).expect("Failed to load Haskell grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return ExtractionResult::new(vec![], vec![], true),
    };

    let root = tree.root_node();
    let src = source.as_bytes();
    let has_errors = root.has_error();

    let scope_tree = scope_tree::build(root, src, HASKELL_SCOPE_KINDS);

    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    // Module header (optional)
    extract_module_header(root, src, &mut symbols);

    visit(root, src, &scope_tree, &mut symbols, &mut refs, None, false);

    ExtractionResult::new(symbols, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Module header  →  Namespace
// ---------------------------------------------------------------------------

fn extract_module_header(root: Node, src: &[u8], symbols: &mut Vec<ExtractedSymbol>) {
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "header" {
            // header → module module_id exports? where
            let mut hcursor = child.walk();
            for hchild in child.children(&mut hcursor) {
                if hchild.kind() == "module" {
                    let name = node_text(hchild, src);
                    if !name.is_empty() {
                        symbols.push(make_symbol(
                            name.clone(),
                            name.clone(),
                            SymbolKind::Namespace,
                            &hchild,
                            Some(format!("module {}", name)),
                            None,
                        ));
                    }
                }
            }
            break;
        }
    }
}

// ---------------------------------------------------------------------------
// Core traversal
// ---------------------------------------------------------------------------

fn visit(
    node: Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    inside_class_or_instance: bool,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function" => {
                let kind = if inside_class_or_instance {
                    SymbolKind::Method
                } else {
                    SymbolKind::Function
                };
                let idx = extract_function(&child, src, scope_tree, symbols, kind, parent_index);
                visit(child, src, scope_tree, symbols, refs, idx.or(parent_index), inside_class_or_instance);
            }
            "data_type" | "data_family" => {
                let idx = extract_named_symbol(
                    &child, src, scope_tree, symbols, SymbolKind::Struct,
                    "data", parent_index,
                );
                // Extract deriving → Implements
                extract_deriving(&child, src, idx, refs);
                // Extract data_constructor children → EnumMember symbols
                extract_data_constructors(&child, src, idx, symbols);
                visit(child, src, scope_tree, symbols, refs, idx.or(parent_index), false);
            }
            "newtype" => {
                let idx = extract_named_symbol(
                    &child, src, scope_tree, symbols, SymbolKind::Struct,
                    "newtype", parent_index,
                );
                extract_deriving(&child, src, idx, refs);
                visit(child, src, scope_tree, symbols, refs, idx.or(parent_index), false);
            }
            "class" => {
                let idx = extract_named_symbol(
                    &child, src, scope_tree, symbols, SymbolKind::Interface,
                    "class", parent_index,
                );
                // Recurse into class body — methods inside are Method kind
                visit(child, src, scope_tree, symbols, refs, idx.or(parent_index), true);
            }
            "instance" => {
                let idx = extract_instance(&child, src, scope_tree, symbols, refs, parent_index);
                visit(child, src, scope_tree, symbols, refs, idx.or(parent_index), true);
            }
            "type_synomym" | "type_family" => {
                let _ = extract_named_symbol(
                    &child, src, scope_tree, symbols, SymbolKind::TypeAlias,
                    "type", parent_index,
                );
            }
            "import" => {
                extract_import(&child, src, symbols, refs, parent_index);
            }
            "apply" => {
                extract_apply(&child, src, symbols, refs, parent_index);
                visit(child, src, scope_tree, symbols, refs, parent_index, inside_class_or_instance);
            }
            "infix" => {
                extract_infix(&child, src, symbols, refs, parent_index);
                visit(child, src, scope_tree, symbols, refs, parent_index, inside_class_or_instance);
            }
            "foreign_import" | "foreign_export" => {
                extract_foreign(&child, src, scope_tree, symbols, parent_index);
            }
            _ => {
                visit(child, src, scope_tree, symbols, refs, parent_index, inside_class_or_instance);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// function  →  Function or Method
// ---------------------------------------------------------------------------

fn extract_function(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    kind: SymbolKind,
    parent_index: Option<usize>,
) -> Option<usize> {
    // `name` field is optional in tree-sitter-haskell.
    // Try: name field → first variable/prefix_id child → first match child's name.
    let name = if let Some(n) = node.child_by_field_name("name") {
        let t = node_text(n, src);
        t.trim_matches(|c: char| c == '(' || c == ')').to_string()
    } else {
        // Try direct variable or prefix_id child
        let mut cursor = node.walk();
        let mut found = String::new();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "variable" | "prefix_id" => {
                    found = node_text(child, src)
                        .trim_matches(|c: char| c == '(' || c == ')')
                        .to_string();
                    break;
                }
                "match" => {
                    // match node contains the function name as first child
                    let mut mc = child.walk();
                    for mc_child in child.children(&mut mc) {
                        if mc_child.kind() == "variable" || mc_child.kind() == "prefix_id" {
                            found = node_text(mc_child, src)
                                .trim_matches(|c: char| c == '(' || c == ')')
                                .to_string();
                            break;
                        }
                    }
                    if !found.is_empty() {
                        break;
                    }
                }
                _ => {}
            }
        }
        found
    };
    let name = if !name.is_empty() {
        name
    } else {
        // Final fallback: use raw text of the first named child (truncated).
        // This handles pattern-only bindings like `(x, y) = ...` or `_ = ...`.
        let fallback = node.named_child(0)
            .map(|c| {
                let t = node_text(c, src);
                // Truncate to 40 chars to avoid huge names
                if t.len() > 40 { t[..40].to_string() } else { t }
            })
            .unwrap_or_default();
        if fallback.is_empty() {
            return None;
        }
        fallback
    };

    let scope = scope_tree::find_enclosing_scope(scope_tree, node.start_byte(), node.end_byte()).map(|s| s.qualified_name.clone());
    let qname = if let Some(p) = &scope { format!("{}.{}", p, name) } else { name.clone() };

    let idx = symbols.len();
    symbols.push(make_symbol(
        name,
        qname,
        kind,
        node,
        None,
        parent_index,
    ));
    // Attach scope_path
    if let Some(ref s) = scope {
        symbols[idx].scope_path = Some(s.clone());
    }
    Some(idx)
}

// ---------------------------------------------------------------------------
// data_type / newtype / class / type_synomym  →  named symbol
// ---------------------------------------------------------------------------

fn extract_named_symbol(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    kind: SymbolKind,
    keyword: &str,
    parent_index: Option<usize>,
) -> Option<usize> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, src);
    if name.is_empty() {
        return None;
    }

    let scope = scope_tree::find_enclosing_scope(scope_tree, node.start_byte(), node.end_byte()).map(|s| s.qualified_name.clone());
    let qname = if let Some(p) = &scope { format!("{}.{}", p, name) } else { name.clone() };

    let idx = symbols.len();
    symbols.push(make_symbol(
        name.clone(),
        qname,
        kind,
        node,
        Some(format!("{} {} ...", keyword, name)),
        parent_index,
    ));
    if let Some(ref s) = scope {
        symbols[idx].scope_path = Some(s.clone());
    }
    Some(idx)
}

// ---------------------------------------------------------------------------
// instance  →  Class + Implements edge
// ---------------------------------------------------------------------------

fn extract_instance(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) -> Option<usize> {
    // instance [context =>] ClassName Type where ...
    // The type class name is in the `name` field
    let class_name_node = node.child_by_field_name("name")?;
    let class_name = node_text(class_name_node, src);
    if class_name.is_empty() {
        return None;
    }

    // The type being instantiated is in `patterns` or surrounding children
    let type_name = extract_instance_type(node, src);
    let instance_name = if type_name.is_empty() {
        class_name.clone()
    } else {
        format!("{} {}", class_name, type_name)
    };

    let scope = scope_tree::find_enclosing_scope(scope_tree, node.start_byte(), node.end_byte()).map(|s| s.qualified_name.clone());
    let idx = symbols.len();
    symbols.push(make_symbol(
        instance_name.clone(),
        instance_name,
        SymbolKind::Class,
        node,
        Some(format!("instance {} {}", class_name, type_name)),
        parent_index,
    ));
    if let Some(ref s) = scope {
        symbols[idx].scope_path = Some(s.clone());
    }

    // Implements edge: this type instance → the type class
    let source_idx = idx;
    refs.push(ExtractedRef {
        source_symbol_index: source_idx,
        target_name: class_name,
        kind: EdgeKind::Implements,
        line: node.start_position().row as u32,
        module: None,
        chain: None,
    });

    Some(idx)
}

fn extract_instance_type(node: &Node, src: &[u8]) -> String {
    // Walk children after `name` field to find type identifiers
    let mut found_name = false;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if node.child_by_field_name("name").map(|n| n.id()) == Some(child.id()) {
            found_name = true;
            continue;
        }
        if found_name && (child.kind() == "name" || child.kind() == "constructor" || child.kind() == "variable") {
            let t = node_text(child, src);
            if !t.is_empty() {
                return t;
            }
        }
    }
    String::new()
}

// ---------------------------------------------------------------------------
// deriving  →  Implements edges
// ---------------------------------------------------------------------------

fn extract_deriving(
    node: &Node,
    src: &[u8],
    parent_idx: Option<usize>,
    refs: &mut Vec<ExtractedRef>,
) {
    let source_idx = match parent_idx {
        Some(i) => i,
        None => return,
    };
    // Look for `deriving` child node
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "deriving" {
            // Collect all class names from the deriving clause.
            // tree-sitter-haskell grammar (v0.25+) wraps the list in a `tuple`
            // node (for `deriving (Show, Eq)`) or emits a single `name`/`constructor`
            // directly (for `deriving Show`).
            collect_deriving_names(&child, src, source_idx, refs);
        }
    }
}

/// Recursively collect `name`/`constructor`/`class` tokens from a `deriving` node
/// or any of its container wrappers (`tuple`, `list`, `class`, etc.).
fn collect_deriving_names(
    node: &Node,
    src: &[u8],
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "name" | "constructor" => {
                let name = node_text(child, src);
                if !name.is_empty() && name != "deriving" {
                    refs.push(ExtractedRef {
                        source_symbol_index: source_idx,
                        target_name: name,
                        kind: EdgeKind::Implements,
                        line: child.start_position().row as u32,
                        module: None,
                        chain: None,
                    });
                }
            }
            "deriving" | "tuple" | "list" | "class" | "qualified" => {
                // Recurse into wrapper nodes
                collect_deriving_names(&child, src, source_idx, refs);
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// data_constructor children  →  EnumMember symbols
// ---------------------------------------------------------------------------

fn extract_data_constructors(
    data_node: &Node,
    src: &[u8],
    parent_idx: Option<usize>,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    // Walk all descendants looking for `data_constructor` or `gadt_constructor` nodes.
    collect_constructors(data_node, src, parent_idx, symbols);
}

fn collect_constructors(
    node: &Node,
    src: &[u8],
    parent_idx: Option<usize>,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "data_constructor" | "gadt_constructor" => {
                // The constructor name is inside a `prefix` child (or a direct
                // `constructor`/`name` child in some grammar versions).
                // Use a depth-first search limited to 3 levels to find the first
                // `constructor` node.
                if let Some(name) = find_constructor_name(&child, src, 3) {
                    symbols.push(make_symbol(
                        name.clone(),
                        name,
                        SymbolKind::EnumMember,
                        &child,
                        None,
                        parent_idx,
                    ));
                }
            }
            _ => {
                collect_constructors(&child, src, parent_idx, symbols);
            }
        }
    }
}

/// Depth-limited search for a `constructor` or `name` node inside a data/gadt
/// constructor node. Returns the text of the first match found.
fn find_constructor_name(node: &Node, src: &[u8], depth: usize) -> Option<String> {
    if depth == 0 {
        return None;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "constructor" {
            let name = node_text(child, src);
            if !name.is_empty() {
                return Some(name);
            }
        }
        // Recurse into wrapper nodes like `prefix`, `infix_constructor`
        if let Some(name) = find_constructor_name(&child, src, depth - 1) {
            return Some(name);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// import  →  Imports edge
// ---------------------------------------------------------------------------

fn extract_import(
    node: &Node,
    src: &[u8],
    symbols: &[ExtractedSymbol],
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let source_idx = parent_index.unwrap_or_else(|| symbols.len().saturating_sub(1));
    // module field = the module being imported
    let module_node = match node.child_by_field_name("module") {
        Some(n) => n,
        None => return,
    };
    let module = node_text(module_node, src);
    if module.is_empty() {
        return;
    }
    refs.push(ExtractedRef {
        source_symbol_index: source_idx,
        target_name: module.rsplit('.').next().unwrap_or(&module).to_string(),
        kind: EdgeKind::Imports,
        line: node.start_position().row as u32,
        module: Some(module),
        chain: None,
    });
}

// ---------------------------------------------------------------------------
// apply  →  Calls edge
// ---------------------------------------------------------------------------

fn extract_apply(
    node: &Node,
    src: &[u8],
    symbols: &[ExtractedSymbol],
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let source_idx = parent_index.unwrap_or_else(|| symbols.len().saturating_sub(1));
    // `function` field is optional in tree-sitter-haskell; fall back to first named child.
    let (fname, fmodule) = if let Some(func_node) = node.child_by_field_name("function") {
        extract_apply_target(func_node, src)
    } else {
        // Walk children to find the function expression
        let mut result = (String::new(), None);
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                if !child.is_named() {
                    continue;
                }
                match child.kind() {
                    "variable" | "name" | "constructor" | "qualified" | "prefix_id"
                    | "operator" | "operator_name" | "apply" | "parenthesized_expression" => {
                        let (t, m) = extract_apply_target(child, src);
                        if !t.is_empty() {
                            result = (t, m);
                            break;
                        }
                    }
                    "expression" => {
                        // Unwrap expression wrapper
                        for j in 0..child.child_count() {
                            if let Some(gc) = child.child(j) {
                                if gc.is_named() {
                                    let (t, m) = extract_apply_target(gc, src);
                                    if !t.is_empty() {
                                        result = (t, m);
                                        break;
                                    }
                                }
                            }
                        }
                        if !result.0.is_empty() {
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }
        result
    };
    if fname.is_empty() {
        return;
    }
    refs.push(ExtractedRef {
        source_symbol_index: source_idx,
        target_name: fname,
        kind: EdgeKind::Calls,
        line: node.start_position().row as u32,
        module: fmodule,
        chain: None,
    });
}

/// Returns `(function_name, module_qualifier)` for the given callee node.
fn extract_apply_target(node: Node, src: &[u8]) -> (String, Option<String>) {
    match node.kind() {
        "variable" | "name" | "constructor" | "operator" | "operator_name" | "prefix_id" => {
            let name = node_text(node, src)
                .trim_matches(|c: char| c == '(' || c == ')' || c == '`')
                .to_string();
            (name, None)
        }
        "qualified" => {
            // tree-sitter-haskell `qualified` has named fields `module` and `id`.
            // `module` contains a `module` node whose text is the full qualifier
            // (e.g. "Data.Map" for `Data.Map.lookup`).
            // `id` is the final function name.
            let id = node.child_by_field_name("id")
                .map(|n| node_text(n, src))
                .unwrap_or_else(|| {
                    let count = node.named_child_count();
                    if count > 0 {
                        node.named_child(count - 1)
                            .map(|n| node_text(n, src))
                            .unwrap_or_default()
                    } else {
                        String::new()
                    }
                });
            // The `module` field node spans the qualifier including the trailing
            // `.` separator (e.g. "Map." or "Data.Map."). Strip the trailing dot.
            let module = node.child_by_field_name("module")
                .map(|n| node_text(n, src).trim_end_matches('.').to_string())
                .filter(|s| !s.is_empty());
            (id, module)
        }
        "apply" => {
            // Nested apply (curried) — recurse to find the base function
            node.child_by_field_name("function")
                .map(|n| extract_apply_target(n, src))
                .unwrap_or_default()
        }
        "parenthesized_expression" => {
            // Could be a section like `(+3)` or `(f)` — try first named child
            node.named_child(0)
                .map(|n| extract_apply_target(n, src))
                .unwrap_or_default()
        }
        _ => (String::new(), None),
    }
}

// ---------------------------------------------------------------------------
// infix  →  Calls edge
// ---------------------------------------------------------------------------

fn extract_infix(
    node: &Node,
    src: &[u8],
    symbols: &[ExtractedSymbol],
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let source_idx = parent_index.unwrap_or_else(|| symbols.len().saturating_sub(1));
    // infix: left operator right
    // The operator field holds the infix function name (backtick or operator)
    let op_node = node.child_by_field_name("operator")
        .or_else(|| {
            // Fallback: find the operator child — check all children (named or anonymous)
            let count = node.child_count();
            if count >= 3 {
                // Middle child (index 1 in 3-child infix: left op right)
                node.child(1)
            } else {
                // Second named child fallback
                let mut cursor = node.walk();
                let children: Vec<Node> = node.children(&mut cursor).collect();
                let named: Vec<Node> = children.into_iter().filter(|c| c.is_named()).collect();
                if named.len() >= 2 { Some(named[1]) } else { None }
            }
        });

    let op_text = op_node
        .map(|n| {
            let t = node_text(n, src);
            // Strip backtick quoting from infix functions like `elem`
            t.trim_matches('`').to_string()
        })
        .unwrap_or_default();

    if op_text.is_empty() {
        return;
    }

    refs.push(ExtractedRef {
        source_symbol_index: source_idx,
        target_name: op_text,
        kind: EdgeKind::Calls,
        line: node.start_position().row as u32,
        module: None,
        chain: None,
    });
}

// ---------------------------------------------------------------------------
// foreign_import / foreign_export  →  Function
// ---------------------------------------------------------------------------

fn extract_foreign(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    // foreign_import / foreign_export contains a `signature` child
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "signature" {
            let name_node = child.child_by_field_name("name")
                .or_else(|| child.named_child(0))?;
            let name = node_text(name_node, src);
            if name.is_empty() {
                return None;
            }
            let scope = scope_tree::find_enclosing_scope(scope_tree, node.start_byte(), node.end_byte()).map(|s| s.qualified_name.clone());
            let qname = if let Some(p) = &scope { format!("{}.{}", p, name) } else { name.clone() };
            let idx = symbols.len();
            symbols.push(make_symbol(name, qname, SymbolKind::Function, node, None, parent_index));
            if let Some(ref s) = scope {
                symbols[idx].scope_path = Some(s.clone());
            }
            return Some(idx);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_symbol(
    name: String,
    qualified_name: String,
    kind: SymbolKind,
    node: &Node,
    signature: Option<String>,
    parent_index: Option<usize>,
) -> ExtractedSymbol {
    ExtractedSymbol {
        name,
        qualified_name,
        kind,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature,
        doc_comment: None,
        scope_path: None,
        parent_index,
    }
}

fn node_text(node: Node, src: &[u8]) -> String {
    std::str::from_utf8(&src[node.start_byte()..node.end_byte()])
        .unwrap_or("")
        .to_string()
}
