// =============================================================================
// languages/gdscript/extract.rs  —  GDScript symbol and reference extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Class      — `class_name_statement`, `class_definition` (inner)
//   Function   — `function_definition` (top-level without class)
//   Method     — `function_definition` (inside class_definition)
//   Event      — `signal_statement`
//   Property   — `export_variable_statement`
//   Field      — `variable_statement` inside class / `onready_variable_statement`
//   Variable   — `variable_statement` at top-level
//   Constant   — `const_statement`
//   Enum       — `enum_definition`
//
// REFERENCES:
//   Inherits   — `class_name_statement.extends` / `extends_statement`
//   Calls      — `call` nodes
// =============================================================================

use crate::types::{EdgeKind, ExtractionResult, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::{Node, Parser};

pub fn extract(source: &str) -> ExtractionResult {
    let language: tree_sitter::Language = tree_sitter_gdscript::LANGUAGE.into();
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

    visit(tree.root_node(), source, &mut symbols, &mut refs, None, false);

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
    inside_class: bool,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_name_statement" => {
                extract_class_name_stmt(&child, src, symbols, refs, parent_index);
            }
            "extends_statement" => {
                extract_extends_stmt(&child, src, parent_index.unwrap_or(0), refs);
            }
            "class_definition" => {
                extract_inner_class(&child, src, symbols, refs, parent_index);
            }
            "function_definition" => {
                extract_function(&child, src, symbols, refs, parent_index, inside_class);
            }
            "constructor_definition" => {
                extract_constructor(&child, src, symbols, refs, parent_index);
            }
            "signal_statement" => {
                extract_signal(&child, src, symbols, parent_index);
            }
            "export_variable_statement" => {
                extract_export_var(&child, src, symbols, parent_index);
            }
            "variable_statement" | "onready_variable_statement" => {
                extract_variable(&child, src, symbols, refs, parent_index, inside_class);
            }
            "const_statement" => {
                extract_const(&child, src, symbols, refs, parent_index);
            }
            "enum_definition" => {
                extract_enum(&child, src, symbols, parent_index);
            }
            _ => {
                visit(child, src, symbols, refs, parent_index, inside_class);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// class_name_statement: top-level class declaration
// ---------------------------------------------------------------------------

fn extract_class_name_stmt(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let name = match node.child_by_field_name("name") {
        Some(n) => node_text(&n, src).to_string(),
        None => return,
    };

    let line = node.start_position().row as u32;
    let idx = symbols.len();

    // The `extends` field on `class_name_statement` carries the entire
    // `extends X` clause text, not just the bare type name. Strip the
    // `extends` keyword before treating the value as an Inherits target,
    // otherwise the literal `extends PandoraEntity` ends up in the index
    // and never resolves.
    let extends = node.child_by_field_name("extends").map(|n| {
        let text = node_text(&n, src);
        text.trim()
            .trim_start_matches("extends")
            .trim()
            .to_string()
    });

    let sig = match &extends {
        Some(base) => format!("class_name {} extends {}", name, base),
        None => format!("class_name {}", name),
    };

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Class,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: line,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(sig),
        doc_comment: None,
        scope_path: None,
        parent_index,
    });

    if let Some(base) = extends {
        if base.is_empty() {
            return;
        }
        refs.push(ExtractedRef {
            source_symbol_index: idx,
            target_name: base,
            kind: EdgeKind::Inherits,
            line,
            module: None,
            chain: None,
            byte_offset: 0,
                    namespace_segments: Vec::new(),
});
    }
}

// ---------------------------------------------------------------------------
// extends_statement: standalone `extends SomeClass`
// ---------------------------------------------------------------------------

fn extract_extends_stmt(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Trim surrounding whitespace BEFORE stripping the keyword. The raw
    // node text can carry leading indentation (`    extends RefCounted`)
    // when an `extends_statement` appears inside an inner class, in which
    // case `.trim_start_matches("extends")` won't match and the literal
    // `extends X` text ends up as the inherits target.
    let text = node_text(node, src);
    let base = text
        .trim()
        .trim_start_matches("extends")
        .trim()
        .to_string();
    if base.is_empty() {
        return;
    }
    refs.push(ExtractedRef {
        source_symbol_index,
        target_name: base,
        kind: EdgeKind::Inherits,
        line: node.start_position().row as u32,
        module: None,
        chain: None,
        byte_offset: 0,
            namespace_segments: Vec::new(),
});
}

// ---------------------------------------------------------------------------
// Inner class definition
// ---------------------------------------------------------------------------

fn extract_inner_class(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let name = match node.child_by_field_name("name") {
        Some(n) => node_text(&n, src).to_string(),
        None => return,
    };

    let line = node.start_position().row as u32;
    let idx = symbols.len();

    let extends = node.child_by_field_name("extends").map(|n| {
        let text = node_text(&n, src);
        text.trim()
            .trim_start_matches("extends")
            .trim()
            .to_string()
    });

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Class,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(format!("class {}", name)),
        doc_comment: None,
        scope_path: None,
        parent_index,
    });

    if let Some(base) = extends {
        if !base.is_empty() {
            refs.push(ExtractedRef {
                source_symbol_index: idx,
                target_name: base,
                kind: EdgeKind::Inherits,
                line,
                module: None,
                chain: None,
                byte_offset: 0,
                namespace_segments: Vec::new(),
            });
        }
    }

    // Walk class body
    visit(*node, src, symbols, refs, Some(idx), true);
}

// ---------------------------------------------------------------------------
// Function / Method
// ---------------------------------------------------------------------------

fn extract_function(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    inside_class: bool,
) {
    let name = match node.child_by_field_name("name") {
        Some(n) => node_text(&n, src).to_string(),
        None => return,
    };

    let kind = if inside_class { SymbolKind::Method } else { SymbolKind::Function };
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
        signature: Some(format!("func {}", name)),
        doc_comment: None,
        scope_path: None,
        parent_index,
    });

    collect_calls(node, src, idx, refs);
    // Walk function body for nested variable declarations, const, and enum nodes.
    visit(*node, src, symbols, refs, Some(idx), true);
}

fn extract_constructor(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let line = node.start_position().row as u32;
    let idx = symbols.len();

    symbols.push(ExtractedSymbol {
        name: "_init".to_string(),
        qualified_name: "_init".to_string(),
        kind: SymbolKind::Constructor,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some("func _init()".to_string()),
        doc_comment: None,
        scope_path: None,
        parent_index,
    });

    collect_calls(node, src, idx, refs);
    // Walk constructor body for nested variable declarations.
    visit(*node, src, symbols, refs, Some(idx), true);
}

// ---------------------------------------------------------------------------
// Signal → Event
// ---------------------------------------------------------------------------

fn extract_signal(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    let name = match node.child_by_field_name("name") {
        Some(n) => node_text(&n, src).to_string(),
        None => return,
    };

    let line = node.start_position().row as u32;

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Event,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: line,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(format!("signal {}", name)),
        doc_comment: None,
        scope_path: None,
        parent_index,
    });
}

// ---------------------------------------------------------------------------
// @export variable → Property
// ---------------------------------------------------------------------------

fn extract_export_var(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    let name = match node.child_by_field_name("name") {
        Some(n) => node_text(&n, src).to_string(),
        None => return,
    };

    let line = node.start_position().row as u32;

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Property,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: line,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(format!("@export var {}", name)),
        doc_comment: None,
        scope_path: None,
        parent_index,
    });
}

// ---------------------------------------------------------------------------
// Variable statement
// ---------------------------------------------------------------------------

fn extract_variable(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    inside_class: bool,
) {
    let name = match node.child_by_field_name("name") {
        Some(n) => node_text(&n, src).to_string(),
        None => return,
    };

    // Check if this variable_statement has an `@export` annotation — if so,
    // treat it as a Property (the grammar emits variable_statement with an
    // `annotations` child rather than a separate export_variable_statement node).
    let is_export = has_export_annotation(node, src);

    let kind = if is_export {
        SymbolKind::Property
    } else if inside_class {
        SymbolKind::Field
    } else {
        SymbolKind::Variable
    };

    let sig = if is_export {
        format!("@export var {}", name)
    } else {
        format!("var {}", name)
    };

    let line = node.start_position().row as u32;
    let idx = symbols.len();

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: line,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(sig),
        doc_comment: None,
        scope_path: None,
        parent_index,
    });

    // Walk the initializer for calls so `preload(...)` / `load(...)` on the
    // RHS emits its Imports edge via collect_calls.
    collect_calls(node, src, idx, refs);
}

/// Return true if the variable_statement node has an `@export` annotation.
/// The grammar wraps annotations in an `annotations` child containing one or
/// more `annotation` nodes.  Each annotation starts with `@` followed by an
/// `identifier` (e.g. "export").
fn has_export_annotation(node: &Node, src: &str) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "annotations" {
            let mut ac = child.walk();
            for ann in child.children(&mut ac) {
                if ann.kind() == "annotation" {
                    // Look for an identifier child with text "export"
                    let mut ic = ann.walk();
                    for ann_child in ann.children(&mut ic) {
                        if ann_child.kind() == "identifier"
                            && node_text(&ann_child, src) == "export"
                        {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Constant
// ---------------------------------------------------------------------------

fn extract_const(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let name = match node.child_by_field_name("name") {
        Some(n) => node_text(&n, src).to_string(),
        None => return,
    };

    let line = node.start_position().row as u32;
    let idx = symbols.len();

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Variable,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: line,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(format!("const {}", name)),
        doc_comment: None,
        scope_path: None,
        parent_index,
    });

    // `const Foo := preload(...)` — walk the initializer so the preload call
    // emits its Imports edge via collect_calls.
    collect_calls(node, src, idx, refs);
}

// ---------------------------------------------------------------------------
// Enum
// ---------------------------------------------------------------------------

fn extract_enum(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    // Anonymous enums (`enum { A, B }`) have no name field — use a placeholder.
    let name = node.child_by_field_name("name")
        .map(|n| node_text(&n, src).to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "<anonymous_enum>".to_string());

    let line = node.start_position().row as u32;

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Enum,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(format!("enum {} {{ ... }}", name)),
        doc_comment: None,
        scope_path: None,
        parent_index,
    });
}

// ---------------------------------------------------------------------------
// Collect call nodes in subtree
// ---------------------------------------------------------------------------

fn collect_calls(node: &Node, src: &str, source_idx: usize, refs: &mut Vec<ExtractedRef>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call" {
            // callee is the first expression child
            if let Some(callee) = child.child(0) {
                let name = match callee.kind() {
                    "identifier" | "attribute_call" => node_text(&callee, src).to_string(),
                    _ => String::new(),
                };
                if !name.is_empty() {
                    let line = child.start_position().row as u32;
                    refs.push(ExtractedRef {
                        source_symbol_index: source_idx,
                        target_name: name.clone(),
                        kind: EdgeKind::Calls,
                        line,
                        module: None,
                        chain: None,
                        byte_offset: 0,
                                            namespace_segments: Vec::new(),
});

                    // `preload("res://path/to/foo.gd")` and `load(...)` bring another
                    // script/scene file into scope. The call itself is a builtin that
                    // resolves to nothing useful, but the path argument is a cross-
                    // file import — emit an Imports edge so the resolver can link
                    // symbols defined in the target .gd file. Mirrors the bash
                    // `source`/`.` precedent in cb2fc80.
                    if name == "preload" || name == "load" {
                        if let Some(raw) = first_string_arg(&child, src) {
                            // Strip `res://` (Godot project-root prefix); keep
                            // relative paths as-is.
                            let normalized = raw.trim_start_matches("res://").to_string();
                            let target = normalized
                                .rsplit('/')
                                .next()
                                .unwrap_or(&normalized)
                                .trim_end_matches(".gd")
                                .trim_end_matches(".tscn")
                                .trim_end_matches(".tres")
                                .to_string();
                            if !target.is_empty() {
                                refs.push(ExtractedRef {
                                    source_symbol_index: source_idx,
                                    target_name: target,
                                    kind: EdgeKind::Imports,
                                    line,
                                    module: Some(normalized),
                                    chain: None,
                                    byte_offset: 0,
                                                                    namespace_segments: Vec::new(),
});
                            }
                        }
                    }
                }
            }
        }
        collect_calls(&child, src, source_idx, refs);
    }
}

/// Return the text of the first string literal appearing in this call node's
/// argument list, with surrounding quotes trimmed.
fn first_string_arg(call_node: &Node, src: &str) -> Option<String> {
    let mut cursor = call_node.walk();
    for child in call_node.children(&mut cursor) {
        // tree-sitter-gdscript wraps call args in `arguments`; scan its children.
        if child.kind() == "arguments" {
            let mut inner = child.walk();
            for arg in child.children(&mut inner) {
                if arg.kind() == "string" {
                    let raw = node_text(&arg, src);
                    let trimmed = raw.trim_matches('"').trim_matches('\'');
                    if !trimmed.is_empty() {
                        return Some(trimmed.to_string());
                    }
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn node_text<'a>(node: &Node, src: &'a str) -> &'a str {
    node.utf8_text(src.as_bytes()).unwrap_or("")
}
