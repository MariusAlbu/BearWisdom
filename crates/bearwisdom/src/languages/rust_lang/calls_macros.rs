// =============================================================================
// rust/calls_macros.rs  —  Macro-argument re-parse for nested call extraction
// =============================================================================

use super::helpers::node_text;
use crate::types::{EdgeKind, ExtractedRef};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// Macro argument re-parse — extract nested calls from `token_tree` contents
// ---------------------------------------------------------------------------

/// Re-parse the contents of a `macro_invocation`'s argument `token_tree` as
/// a Rust expression and emit `Calls` edges for every `call_expression` we
/// find inside. Handles `()`, `[]`, and `{}` macro delimiter styles by
/// stripping the outer punctuation and wrapping the body in a synthetic
/// `fn _macro_arg() { ( <body> ); }` so multi-comma argument lists parse
/// as a tuple. When the inner text fails to parse cleanly (rare — `cfg!`
/// and similar non-expression DSLs), tree-sitter still produces a partial
/// AST; we walk what's there and skip what's not.
pub(super) fn extract_calls_from_macro_args(
    macro_node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Find the token-tree child. Tree-sitter-rust may name it differently
    // depending on the delimiter, but it's always the last named child.
    let mut tt_node: Option<Node> = None;
    let mut walker = macro_node.walk();
    for c in macro_node.children(&mut walker) {
        let kind = c.kind();
        if kind == "token_tree" || kind.contains("token_tree") {
            tt_node = Some(c);
        }
    }
    let Some(tt) = tt_node else { return };

    // Strip the outer delimiter pair from the token_tree text. The first
    // and last bytes are always single-byte ASCII punctuation: '(' / ')',
    // '[' / ']', or '{' / '}'.
    let tt_text = node_text(&tt, source);
    if tt_text.len() < 2 {
        return;
    }
    let inner = &tt_text[1..tt_text.len() - 1];
    if inner.trim().is_empty() {
        return;
    }

    // Wrap as a tuple expression statement so multi-arg macros parse:
    //   `assert_eq!(a, b)` → `( a, b )` → `(tuple)`. Single-arg cases
    //   `( expr )` parse as parenthesized expression.
    let wrapped = format!("fn _macro_arg() {{ ({inner}); }}");
    let mut parser = tree_sitter::Parser::new();
    if parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .is_err()
    {
        return;
    }
    let Some(tree) = parser.parse(&wrapped, None) else {
        return;
    };

    // Walk the synthetic AST and collect call_expression / scoped_identifier
    // function targets. Attribute every nested call to the original
    // macro_invocation's start line — we lose precise per-call line info
    // because the synthetic source has different positions, but resolution
    // doesn't need exact lines (it uses scope chain + source symbol).
    let macro_line = macro_node.start_position().row as u32;
    let macro_byte = macro_node.start_byte() as u32;
    walk_synthetic_macro_calls(
        &tree.root_node(),
        &wrapped,
        source_symbol_index,
        refs,
        macro_line,
        macro_byte,
    );
}

fn walk_synthetic_macro_calls(
    node: &Node,
    synthetic_source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
    line: u32,
    byte_offset: u32,
) {
    let mut walker = node.walk();
    for child in node.children(&mut walker) {
        match child.kind() {
            "call_expression" => {
                if let Some(fn_node) = child.child_by_field_name("function") {
                    let raw = node_text(&fn_node, synthetic_source);
                    let raw = raw.trim();
                    if raw.is_empty() {
                        // Method call (foo.bar()) — fn_node is a field_expression.
                        // Skip these; the chain walker handles them at the
                        // primary-AST level. Macro args don't typically need
                        // method-chain tracking at the same depth.
                    } else if let Some((prefix, leaf)) = raw.rsplit_once("::") {
                        if !prefix.is_empty() && !leaf.is_empty() {
                            refs.push(ExtractedRef {
                                source_symbol_index,
                                target_name: leaf.to_string(),
                                kind: EdgeKind::Calls,
                                line,
                                module: Some(prefix.to_string()),
                                chain: None,
                                byte_offset,
                                namespace_segments: Vec::new(),
                                call_args: Vec::new(),
                            });
                        }
                    } else if !raw.contains(['.', '(']) {
                        // Plain identifier call: foo(...).
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name: raw.to_string(),
                            kind: EdgeKind::Calls,
                            line,
                            module: None,
                            chain: None,
                            byte_offset,
                            namespace_segments: Vec::new(),
                            call_args: Vec::new(),
                        });
                    }
                }
                // Continue walking arguments — nested calls like foo(bar()) need both.
                walk_synthetic_macro_calls(
                    &child,
                    synthetic_source,
                    source_symbol_index,
                    refs,
                    line,
                    byte_offset,
                );
            }
            _ => {
                walk_synthetic_macro_calls(
                    &child,
                    synthetic_source,
                    source_symbol_index,
                    refs,
                    line,
                    byte_offset,
                );
            }
        }
    }
}
