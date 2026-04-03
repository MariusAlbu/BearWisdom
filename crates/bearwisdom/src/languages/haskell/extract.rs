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
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, src);
    if name.is_empty() {
        return None;
    }

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
            // deriving contains class names
            let mut dcursor = child.walk();
            for dc in child.children(&mut dcursor) {
                if dc.kind() == "name" || dc.kind() == "constructor" {
                    let name = node_text(dc, src);
                    if !name.is_empty() && !BUILTIN_TYPES.contains(&name.as_str()) {
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
            }
        }
    }
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
    // apply has `function` field
    let func_node = match node.child_by_field_name("function") {
        Some(n) => n,
        None => return,
    };
    let fname = extract_apply_target(func_node, src);
    if fname.is_empty() {
        return;
    }
    refs.push(ExtractedRef {
        source_symbol_index: source_idx,
        target_name: fname,
        kind: EdgeKind::Calls,
        line: node.start_position().row as u32,
        module: None,
        chain: None,
    });
}

fn extract_apply_target(node: Node, src: &[u8]) -> String {
    match node.kind() {
        "variable" | "name" | "constructor" => node_text(node, src),
        "qualified" => {
            // module.id — use the final identifier
            node.child_by_field_name("id")
                .map(|n| node_text(n, src))
                .unwrap_or_default()
        }
        _ => String::new(),
    }
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
