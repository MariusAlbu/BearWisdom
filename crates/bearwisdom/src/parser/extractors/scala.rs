// =============================================================================
// parser/extractors/scala.rs  —  Scala symbol and reference extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Class       — `class Foo`
//   Namespace   — `object Foo` (Scala companion objects / top-level singletons)
//   Interface   — `trait Foo`
//   Enum        — `enum Foo` (Scala 3), `case class Foo` (Scala 2 sealed hierarchies)
//   EnumMember  — `case` entries inside a Scala 3 `enum`
//   Method      — `def foo(…)` inside a class/object/trait
//   Function    — `def foo(…)` at top level (Scala 3 top-level defs)
//   Property    — `val`/`var` declarations
//
// REFERENCES:
//   import_declaration            → EdgeKind::Imports
//   `extends` clause              → EdgeKind::Inherits (first parent)
//   `with` clause                 → EdgeKind::Implements (additional mixins)
//   call expression               → EdgeKind::Calls
//
// Grammar notes (tree-sitter-scala 0.25):
//   class_definition          → name: identifier, type_parameters?, class_parameters*,
//                               extends_clause?, template_body?
//   object_definition         → name: identifier, extends_clause?, template_body?
//   trait_definition          → name: identifier, extends_clause?, template_body?
//   enum_definition           → name: identifier, extends_clause?, enum_body?    (Scala 3)
//   enum_case_definitions     → enum_case_definition* inside enum_body
//   function_definition       → name: identifier, type_parameters?, parameters*,
//                               return_type?, body?
//   val_definition            → pattern: identifier / typed_pattern, type?, body
//   var_definition            → same
//   import_declaration        → import_selectors? (contains dotted path)
//   extends_clause            → children: `extends`, type_ref+
//   with_clause / with_item   → children: `with`, type_ref+
//   call_expression           → function: expression, arguments?
//   infix_expression          → best-effort; we do not extract these as calls
// =============================================================================

use crate::parser::scope_tree::{self, ScopeKind};
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Scope configuration
// ---------------------------------------------------------------------------

static SCALA_SCOPE_KINDS: &[ScopeKind] = &[
    ScopeKind { node_kind: "class_definition",    name_field: "name" },
    ScopeKind { node_kind: "object_definition",   name_field: "name" },
    ScopeKind { node_kind: "trait_definition",    name_field: "name" },
    ScopeKind { node_kind: "enum_definition",     name_field: "name" },
    ScopeKind { node_kind: "function_definition", name_field: "name" },
];

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Extract all symbols and references from Scala source.
pub fn extract(source: &str) -> super::ExtractionResult {
    let lang: tree_sitter::Language = tree_sitter_scala::LANGUAGE.into();

    let mut parser = Parser::new();
    parser
        .set_language(&lang)
        .expect("Failed to load Scala grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return super::ExtractionResult::new(vec![], vec![], true),
    };

    let root = tree.root_node();
    let src = source.as_bytes();
    let has_errors = root.has_error();

    let scope_tree = scope_tree::build(root, src, SCALA_SCOPE_KINDS);

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

            "class_definition" => {
                let kind = classify_class(&child, src);
                let idx = push_type_def(&child, src, scope_tree, kind, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_extends_with(&child, src, sym_idx, refs);
                }
                recurse_body(&child, src, scope_tree, symbols, refs, idx);
            }

            "object_definition" => {
                let idx = push_type_def(&child, src, scope_tree, SymbolKind::Namespace, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_extends_with(&child, src, sym_idx, refs);
                }
                recurse_body(&child, src, scope_tree, symbols, refs, idx);
            }

            "trait_definition" => {
                let idx = push_type_def(&child, src, scope_tree, SymbolKind::Interface, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_extends_with(&child, src, sym_idx, refs);
                }
                recurse_body(&child, src, scope_tree, symbols, refs, idx);
            }

            // Scala 3 enum
            "enum_definition" => {
                let idx = push_type_def(&child, src, scope_tree, SymbolKind::Enum, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_extends_with(&child, src, sym_idx, refs);
                }
                extract_enum_body(&child, src, scope_tree, symbols, refs, idx);
            }

            "function_definition" => {
                let idx = push_function_def(&child, src, scope_tree, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_calls_from_body(&body, src, sym_idx, refs);
                    }
                }
            }

            "val_definition" | "var_definition" => {
                push_val_var(&child, src, scope_tree, symbols, parent_index);
            }

            "ERROR" | "MISSING" => {}

            _ => {
                extract_node(child, src, scope_tree, symbols, refs, parent_index);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Body recursion helpers
// ---------------------------------------------------------------------------

fn recurse_body(
    type_node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    if let Some(body) = type_node.child_by_field_name("body") {
        extract_node(body, src, scope_tree, symbols, refs, parent_index);
    } else {
        // Scan for template_body or class_body children.
        let mut cursor = type_node.walk();
        for child in type_node.children(&mut cursor) {
            match child.kind() {
                "template_body" | "class_body" => {
                    extract_node(child, src, scope_tree, symbols, refs, parent_index);
                }
                _ => {}
            }
        }
    }
}

fn extract_enum_body(
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
        if child.kind() == "enum_body" {
            let mut cursor = child.walk();
            for item in child.children(&mut cursor) {
                match item.kind() {
                    "enum_case_definitions" => {
                        // enum_case_definitions → enum_case_definition*
                        let mut ic = item.walk();
                        for case_def in item.children(&mut ic) {
                            if case_def.kind() == "enum_case_definition" {
                                if let Some(name_node) = case_def.child_by_field_name("name") {
                                    let name = node_text(name_node, src);
                                    let qualified_name = if enum_qname.is_empty() {
                                        name.clone()
                                    } else {
                                        format!("{enum_qname}.{name}")
                                    };
                                    let scope = enclosing_scope(scope_tree, case_def.start_byte(), case_def.end_byte());
                                    symbols.push(ExtractedSymbol {
                                        name,
                                        qualified_name,
                                        kind: SymbolKind::EnumMember,
                                        visibility: None,
                                        start_line: case_def.start_position().row as u32,
                                        end_line: case_def.end_position().row as u32,
                                        start_col: case_def.start_position().column as u32,
                                        end_col: case_def.end_position().column as u32,
                                        signature: None,
                                        doc_comment: None,
                                        scope_path: scope_tree::scope_path(scope),
                                        parent_index,
                                    });
                                }
                            }
                        }
                    }
                    // Other items in enum body (defs, vals, etc.).
                    _ => {
                        extract_node(item, src, scope_tree, symbols, refs, parent_index);
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Symbol pushers
// ---------------------------------------------------------------------------

fn push_type_def(
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
        SymbolKind::Namespace => "object",
        SymbolKind::Interface => "trait",
        SymbolKind::Enum      => "enum",
        _                     => "class",
    };

    let type_params = node
        .child_by_field_name("type_parameters")
        .map(|tp| node_text(tp, src))
        .unwrap_or_default();

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
        signature: Some(format!("{kw} {name}{type_params}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

fn push_function_def(
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
        .child_by_field_name("parameters")
        .map(|p| node_text(p, src))
        .unwrap_or_default();
    let ret = node
        .child_by_field_name("return_type")
        .map(|r| format!(": {}", node_text(r, src)))
        .unwrap_or_default();
    let signature = Some(format!("def {name}{params}{ret}"));

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

fn push_val_var(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    // val_definition: pattern field or first identifier child.
    let name_opt = node.child_by_field_name("name")
        .map(|n| node_text(n, src))
        .or_else(|| {
            // Pattern may be typed_pattern → identifier.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "identifier" => return Some(node_text(child, src)),
                    "typed_pattern" => {
                        let mut pc = child.walk();
                        for inner in child.children(&mut pc) {
                            if inner.kind() == "identifier" {
                                return Some(node_text(inner, src));
                            }
                        }
                    }
                    _ => {}
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
    let kw = if text.trim_start().starts_with("val") { "val" } else { "var" };
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

// ---------------------------------------------------------------------------
// Import extraction
// ---------------------------------------------------------------------------

fn push_import(
    node: &Node,
    src: &[u8],
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // import_declaration children: `import`, stable_id, import_selectors?
    // stable_id: dotted identifier path
    // Collect all children that form the path.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_expression" => {
                emit_import_expression(&child, src, current_symbol_count, refs);
            }
            "stable_id" | "identifier" => {
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
            }
            _ => {}
        }
    }
}

fn emit_import_expression(
    node: &Node,
    src: &[u8],
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // import_expression → stable_id, import_selectors?
    let mut cursor = node.walk();
    let mut base: Option<String> = None;
    for child in node.children(&mut cursor) {
        match child.kind() {
            "stable_id" | "identifier" => {
                base = Some(node_text(child, src));
            }
            "import_selectors" | "named_imports" => {
                // { Foo, Bar, ... }
                let base_path = base.as_deref().unwrap_or("");
                let mut sc = child.walk();
                for sel in child.children(&mut sc) {
                    if sel.kind() == "import_selector" || sel.kind() == "identifier" {
                        let name_node = sel.child_by_field_name("name")
                            .unwrap_or(sel);
                        let name = node_text(name_node, src);
                        let module = if base_path.is_empty() {
                            name.clone()
                        } else {
                            format!("{base_path}.{name}")
                        };
                        refs.push(ExtractedRef {
                            source_symbol_index: current_symbol_count,
                            target_name: name,
                            kind: EdgeKind::Imports,
                            line: sel.start_position().row as u32,
                            module: Some(module),
                            chain: None,
                        });
                    }
                }
                return;
            }
            _ => {}
        }
    }
    // No selectors — emit for the stable_id itself.
    if let Some(full) = base {
        let target = full.rsplit('.').next().unwrap_or(&full).to_string();
        refs.push(ExtractedRef {
            source_symbol_index: current_symbol_count,
            target_name: target,
            kind: EdgeKind::Imports,
            line: node.start_position().row as u32,
            module: Some(full),
            chain: None,
        });
    }
}

// ---------------------------------------------------------------------------
// Extends / with extraction
// ---------------------------------------------------------------------------

/// Extract `extends T1 with T2 with T3` from a type definition.
///
/// First parent → Inherits, subsequent `with` mixins → Implements.
fn extract_extends_with(
    node: &Node,
    src: &[u8],
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    let mut first_extends = true;

    for child in node.children(&mut cursor) {
        match child.kind() {
            "extends_clause" => {
                // extends_clause: `extends`, type*
                let mut ec = child.walk();
                for type_node in child.children(&mut ec) {
                    let name = type_name_from_node(&type_node, src);
                    if !name.is_empty() {
                        let kind = if first_extends {
                            first_extends = false;
                            EdgeKind::Inherits
                        } else {
                            EdgeKind::Implements
                        };
                        refs.push(ExtractedRef {
                            source_symbol_index: source_idx,
                            target_name: name,
                            kind,
                            line: type_node.start_position().row as u32,
                            module: None,
                            chain: None,
                        });
                    }
                }
            }
            // `with` mixins (Scala 2 style: `extends Base with Mixin`)
            "with_clause" => {
                let mut wc = child.walk();
                for type_node in child.children(&mut wc) {
                    let name = type_name_from_node(&type_node, src);
                    if !name.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index: source_idx,
                            target_name: name,
                            kind: EdgeKind::Implements,
                            line: type_node.start_position().row as u32,
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

/// Extract a simple type name from a type reference node.
fn type_name_from_node(node: &Node, src: &[u8]) -> String {
    match node.kind() {
        "type_identifier" | "identifier" => node_text(*node, src),
        "generic_type" => {
            // generic_type → type_identifier (first child)
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "type_identifier" || child.kind() == "identifier" {
                    return node_text(child, src);
                }
            }
            String::new()
        }
        "compound_type" | "annotated_type" | "with_type" => {
            // Compound: take first type name.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                let n = type_name_from_node(&child, src);
                if !n.is_empty() { return n; }
            }
            String::new()
        }
        _ => String::new(),
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
                // call_expression → function (first child), arguments?
                if let Some(callee) = child.child_by_field_name("function")
                    .or_else(|| child.named_child(0))
                {
                    let name = call_target_name(&callee, src);
                    if !name.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name: name,
                            kind: EdgeKind::Calls,
                            line: callee.start_position().row as u32,
                            module: None,
                            chain: None,
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

/// Decide whether a class_definition is a case class, sealed class, etc.
///
/// case class → treat as Class (the sealed enum pattern is Scala 2-specific;
///              Scala 3 introduces proper `enum`, handled separately).
fn classify_class(node: &Node, src: &[u8]) -> SymbolKind {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "modifiers" {
            let text = node_text(child, src);
            // `case class` is still a Class — case classes are product types.
            // We do not promote them to Enum here; Scala 3 enums are parsed via
            // `enum_definition` which maps to SymbolKind::Enum already.
            let _ = text; // no special treatment currently
        }
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
        "identifier" | "type_identifier" => node_text(*node, src),
        "field_expression" | "select_expression" | "field_access" => {
            // Dotted access: take the field name (last segment).
            node.child_by_field_name("field")
                .or_else(|| node.child_by_field_name("name"))
                .map(|n| node_text(n, src))
                .unwrap_or_default()
        }
        _ => String::new(),
    }
}

fn detect_visibility(node: &Node, src: &[u8]) -> Option<Visibility> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "modifiers" {
            let text = node_text(child, src);
            if text.contains("private")   { return Some(Visibility::Private);   }
            if text.contains("protected") { return Some(Visibility::Protected); }
            if text.contains("public")    { return Some(Visibility::Public);    }
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "scala_tests.rs"]
mod tests;
