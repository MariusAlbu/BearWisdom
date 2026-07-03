// =============================================================================
// rust/calls_macros.rs  —  Macro-argument re-parse for nested call extraction
// =============================================================================

use super::calls::extract_calls_from_body;
use super::helpers::node_text;
use crate::types::ExtractedRef;
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// Macro argument re-parse — extract nested calls from `token_tree` contents
// ---------------------------------------------------------------------------

/// Fixed prefix synthesized ahead of the macro-argument text (see `extract_calls_from_macro_args`).
/// Its byte length anchors the synthetic-to-real byte rebase — keep it in
/// lockstep with the `format!` call that builds the wrapped source.
const WRAPPER_PREFIX: &str = "fn _macro_arg() { (";

/// Re-parse the contents of a `macro_invocation`'s argument `token_tree` as a
/// Rust expression and splice every ref the real extractor finds inside back
/// into `refs`, rebased onto the host file's coordinates. tree-sitter-rust
/// parses macro arguments as an opaque `token_tree` — `assert!(w.poke())`'s
/// `w.poke()` never appears as a `call_expression`/`field_expression` in the
/// primary AST, so without this splice every call, method call, and type
/// reference inside a macro body is invisible to the graph.
///
/// Handles `()`, `[]`, and `{}` macro delimiter styles by stripping the outer
/// punctuation and wrapping the body in a synthetic `fn _macro_arg() { ( body
/// ); }` so multi-comma argument lists parse as a tuple. Macro arguments that
/// aren't expression-shaped (`matches!` patterns, `quote!` token DSLs) don't
/// reparse cleanly — the synthetic tree carries an ERROR node, and this
/// declines to emit anything rather than walk a broken tree.
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
    let wrapped = format!("{WRAPPER_PREFIX}{inner}); }}");
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
    let root = tree.root_node();
    if root.has_error() {
        return;
    }
    let Some(function_item) = root.named_child(0).filter(|n| n.kind() == "function_item") else {
        return;
    };
    let Some(body) = function_item.child_by_field_name("body") else {
        return;
    };

    // Byte-for-byte, `inner` is a verbatim slice of `source` re-inserted at a
    // fixed offset inside `wrapped`, so both the byte and row deltas between
    // synthetic and real coordinates are constant additive shifts.
    let byte_delta = tt.start_byte() as i64 + 1 - WRAPPER_PREFIX.len() as i64;
    let row_delta = tt.start_position().row as u32;

    let mut synthetic_refs: Vec<ExtractedRef> = Vec::new();
    extract_calls_from_body(&body, &wrapped, source_symbol_index, &mut synthetic_refs);

    for mut r in synthetic_refs {
        r.line = r.line.saturating_add(row_delta);
        r.byte_offset = (r.byte_offset as i64 + byte_delta).max(0) as u32;
        refs.push(r);
    }
}
