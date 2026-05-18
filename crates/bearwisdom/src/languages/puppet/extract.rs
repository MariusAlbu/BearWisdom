// =============================================================================
// languages/puppet/extract.rs  —  Puppet infrastructure-as-code extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Class    — class_definition (Puppet class)
//   Class    — defined_resource_type (define <name> — callable resource type)
//   Function — function_declaration (Puppet 4+ function)
//   Function — node_definition (matches specific hosts)
//   Variable — resource_declaration (resource instance)
//
// REFERENCES:
//   Imports + Calls — include_statement (include foo::bar)
//   Imports + Calls — require_statement (require foo::bar)
//   Calls           — function_call
//   Calls           — resource_declaration → resource type
//   Inherits        — class_definition with class_inherits
//
// Grammar: tree-sitter-puppet (not yet in Cargo.toml — ready for when added).
// Puppet uses '::' as namespace separator; qualified names preserve it.
// =============================================================================

use crate::types::{ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::{Node, Parser};

use super::definitions::{
    extract_class_definition, extract_defined_resource_type, extract_function_declaration,
    extract_node_definition,
};
use super::refs::{
    collect_all_function_calls, collect_resource_references, extract_function_call,
    extract_include_or_require, extract_resource_declaration,
};

#[cfg(test)]
#[path = "extract_tests.rs"]
mod tests;

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Extract all symbols and references from a Puppet manifest (.pp).
///
/// Requires the tree-sitter-puppet grammar to be available as `language`.
/// Called by `PuppetPlugin::extract()` once the grammar is wired in.
#[allow(dead_code)]
pub fn extract(source: &str, language: tree_sitter::Language) -> crate::types::ExtractionResult {
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to load Puppet grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return crate::types::ExtractionResult::new(vec![], vec![], true),
    };

    let has_errors = tree.root_node().has_error();
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    visit_manifest(tree.root_node(), source, &mut symbols, &mut refs);

    // Second pass: collect all resource_reference and function_call nodes
    // for ref coverage (catches nodes missed by dispatch_node traversal).
    collect_resource_references(tree.root_node(), source, &mut refs);
    collect_all_function_calls(tree.root_node(), source, &mut refs);

    // Post-filter: drop refs whose target is a locally-bound `$variable`
    // (class / define / function parameter, or lambda block variable) at
    // the ref's line. Puppet's lambda syntax `$xs.each |T $x| { …use $x… }`
    // emits each use of `$x` as a function_call → Calls ref; without this
    // every block variable lands in unresolved_refs. Same for class params:
    // `class foo ($directory) { … use $directory … }` similarly leaks.
    // Mirrors the TS / Kotlin / Scala type-param filter pattern.
    {
        let mut scopes: Vec<(String, u32, u32)> = Vec::new();
        collect_local_var_scopes(tree.root_node(), source, &mut scopes);
        if !scopes.is_empty() {
            refs.retain(|r| {
                !scopes.iter().any(|(name, start, end)| {
                    &r.target_name == name && r.line >= *start && r.line <= *end
                })
            });
        }
    }

    crate::types::ExtractionResult::new(symbols, refs, has_errors)
}

/// Walk the tree and record every `$variable` binding that should be
/// considered local to a declaration. Three sources:
///   * `class_definition` / `defined_resource_type` / `function_declaration`
///     → walk the `parameter_list` child, extract each `$name` from
///     `parameter > expression > variable`.
///   * Same containers → walk the `block` child and collect the LHS `$name`
///     of every `assignment` statement (`$var = expr`). Puppet has
///     function-wide variable scope (no block scoping), so any assignment
///     anywhere in the body puts that variable in scope for the entire
///     declaration. This catches body-locals like `$mod_libs = $apache::…`
///     whose subscript uses (`$mod_libs[$mod]`) are emitted as
///     `resource_reference` Calls refs by the extractor.
///   * `lambda` → direct `variable` children are block params (`|$x, $y|`).
///
/// Each binding is scoped to the enclosing declaration's line range so
/// uses outside (or in sibling scopes) still resolve normally.
fn collect_local_var_scopes(
    node: Node,
    src: &str,
    out: &mut Vec<(String, u32, u32)>,
) {
    let kind = node.kind();
    let is_param_container = matches!(
        kind,
        "class_definition"
            | "defined_resource_type"
            | "function_declaration"
    );
    if is_param_container {
        let start_line = node.start_position().row as u32;
        let end_line = node.end_position().row as u32;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "parameter_list" {
                collect_param_variable_names(child, src, start_line, end_line, out);
            }
            if child.kind() == "block" {
                collect_block_assignment_vars(child, src, start_line, end_line, out);
            }
        }
    }
    // `$xs.each |$x| { … }` is parsed as `iterator_statement` in
    // tree-sitter-puppet (the grammar exposes `lambda` as a sibling kind
    // but Puppet's pipe-delimited block form lands here instead). Block
    // variables are direct `variable` children appearing before the
    // `block` child; walk in order and stop once we hit the body.
    if kind == "iterator_statement" || kind == "lambda" {
        let start_line = node.start_position().row as u32;
        let end_line = node.end_position().row as u32;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "block" {
                break;
            }
            if child.kind() == "variable" {
                let name = node_text(child, src);
                if !name.is_empty() {
                    out.push((name, start_line, end_line));
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_local_var_scopes(child, src, out);
    }
}

#[cfg(test)]
pub(super) fn _test_collect_local_var_scopes(
    node: Node,
    src: &str,
    out: &mut Vec<(String, u32, u32)>,
) {
    collect_local_var_scopes(node, src, out)
}

/// For each `parameter` inside a `parameter_list`, walk the subtree for
/// the first `variable` node and record its text (`$name`).
fn collect_param_variable_names(
    param_list: Node,
    src: &str,
    start_line: u32,
    end_line: u32,
    out: &mut Vec<(String, u32, u32)>,
) {
    let mut cursor = param_list.walk();
    for param in param_list.children(&mut cursor) {
        if param.kind() != "parameter" {
            continue;
        }
        if let Some(name) = first_variable_descendant(&param, src) {
            out.push((name, start_line, end_line));
        }
    }
}

/// Recursively walk a `block` node and record the LHS `$variable` of every
/// `assignment` statement as a body-local binding scoped to [start_line,
/// end_line]. Puppet has function-wide variable scope — an assignment
/// anywhere in the body (`$mod_libs = $apache::mod_libs`) makes that name
/// available throughout the enclosing declaration, including uses inside
/// nested `if`/`elsif`/`else` sub-blocks that tree-sitter emits as
/// `resource_reference` refs (e.g. `$mod_libs[$mod]`).
fn collect_block_assignment_vars(
    block: Node,
    src: &str,
    start_line: u32,
    end_line: u32,
    out: &mut Vec<(String, u32, u32)>,
) {
    let mut cursor = block.walk();
    for stmt in block.children(&mut cursor) {
        match stmt.kind() {
            "assignment" => {
                // assignment: variable ('=' | '+=') expression
                // The first child is the LHS variable.
                if let Some(first) = stmt.child(0) {
                    if first.kind() == "variable" {
                        let name = node_text(first, src);
                        if !name.is_empty() {
                            out.push((name, start_line, end_line));
                        }
                    }
                }
            }
            // Puppet is function-scoped: recurse into nested control-flow
            // blocks so assignments inside `if`/`elsif`/`unless`/`case`
            // sub-blocks are also captured.
            "if_statement" | "unless_statement" | "case_statement" | "elsif_statement"
            | "else_statement" | "case_item" | "default_case" => {
                let mut inner = stmt.walk();
                for child in stmt.children(&mut inner) {
                    if child.kind() == "block" {
                        collect_block_assignment_vars(child, src, start_line, end_line, out);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Pre-order left-first search for the first `variable` descendant.
fn first_variable_descendant(node: &Node, src: &str) -> Option<String> {
    if node.kind() == "variable" {
        let t = node_text(*node, src);
        if !t.is_empty() {
            return Some(t);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(v) = first_variable_descendant(&child, src) {
            return Some(v);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Manifest traversal
// ---------------------------------------------------------------------------

fn visit_manifest(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        dispatch_node(&child, src, symbols, refs, None);
    }
}

pub(super) fn dispatch_node(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    match node.kind() {
        "class_definition" => extract_class_definition(node, src, symbols, refs),
        "defined_resource_type" => extract_defined_resource_type(node, src, symbols, refs),
        "function_declaration" => extract_function_declaration(node, src, symbols, refs),
        "node_definition" => extract_node_definition(node, src, symbols, refs),
        "resource_declaration" => {
            extract_resource_declaration(node, src, symbols, refs, parent_index)
        }
        "include_statement" => extract_include_or_require(node, src, refs, parent_index),
        "require_statement" => extract_include_or_require(node, src, refs, parent_index),
        "function_call" => extract_function_call(node, src, refs, parent_index),
        // Top-level `$var = expr` outside any class/define/function scope —
        // Puppet manifests like `init.pp` and test fixtures put assignments
        // at file scope. Extract them as Variable symbols so subsequent
        // `$var` references in the same file resolve.
        "assignment" => extract_top_level_assignment(node, src, symbols, parent_index),
        _ => {
            // Recurse into block-like containers.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                dispatch_node(&child, src, symbols, refs, parent_index);
            }
        }
    }
}

fn extract_top_level_assignment(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    let Some(first) = node.child(0) else { return };
    if first.kind() != "variable" {
        return;
    }
    let name = node_text(first, src);
    if name.is_empty() {
        return;
    }
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Variable,
        node,
        Some("$<top-level> = ...".to_string()),
        parent_index,
    ));
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

pub(super) fn make_symbol(
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
        byte_offset: node.start_byte() as u32,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
}
}

pub(super) fn node_text(node: Node, src: &str) -> String {
    src[node.start_byte()..node.end_byte()].to_string()
}
