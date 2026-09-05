// =============================================================================
// indexer/flow_bindings — which symbol or ref a flow capture stands for
//
// A flow query names a binding (`@lhs`, `@lhs.param`) and an initializer
// (`@rhs`) by AST node. The rest of the indexer speaks in symbol and ref
// indices, so every capture is correlated here: a binding to the extractor
// symbol it declares, an initializer to the ref whose value the binding takes.
//
// A binding the extractor never emitted as a symbol — a parameter in a
// language whose extractor records only declarations, a local it skipped —
// is synthesized in place: a value symbol under the innermost enclosing
// declaration, so the chain walk can root a member call on it exactly as it
// roots on an extractor-emitted local. External supply never synthesizes:
// its bodies are dropped by the contract filter and a parameter symbol
// carries nothing the callable's own signature does not.
// =============================================================================

use tree_sitter::Node;

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};

/// Whether a binding capture with no extractor symbol gets one synthesized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingSymbols {
    /// Project source: synthesize a value symbol for every uncorrelated binding.
    Synthesize,
    /// External supply: correlate only; an uncorrelated binding is skipped.
    CorrelateOnly,
}

/// Correlate an LHS binding name to its extractor symbol index: the symbol of
/// that name whose start line is closest to (but not after) `line`.
pub(super) fn correlate_lhs_symbol(
    name: &str,
    line: u32,
    symbols: &[ExtractedSymbol],
) -> Option<usize> {
    symbols
        .iter()
        .enumerate()
        .filter(|(_, s)| s.name == name && s.start_line <= line)
        .max_by_key(|(_, s)| s.start_line)
        .map(|(i, _)| i)
}

/// The symbol index a binding capture stands for. A parameter correlates only
/// with a symbol declared on its own line — the signature line — so a
/// same-named field or an earlier function's parameter never absorbs it. A
/// local correlates with the nearest earlier same-name symbol, the extractor's
/// own binding for it. With no correlation and synthesis enabled, a value
/// symbol is appended under the innermost enclosing declaration.
pub(super) fn binding_symbol(
    name: &str,
    node: &Node,
    kind: SymbolKind,
    symbols: &mut Vec<ExtractedSymbol>,
    policy: BindingSymbols,
) -> Option<usize> {
    let line = node.start_position().row as u32;
    let correlated = match kind {
        SymbolKind::Parameter => symbols
            .iter()
            .position(|s| s.name == name && s.start_line == line),
        _ => correlate_lhs_symbol(name, line, symbols),
    };
    match (correlated, policy) {
        (Some(idx), _) => Some(idx),
        (None, BindingSymbols::CorrelateOnly) => None,
        (None, BindingSymbols::Synthesize) => Some(synthesize(name, node, kind, symbols)),
    }
}

/// Append a value symbol for `name` declared at `node`, parented on the
/// innermost extractor symbol whose line span contains it.
fn synthesize(name: &str, node: &Node, kind: SymbolKind, symbols: &mut Vec<ExtractedSymbol>) -> usize {
    let line = node.start_position().row as u32;
    let parent_index = enclosing_symbol(line, symbols);
    let (qualified_name, scope_path) = match parent_index.map(|p| &symbols[p]) {
        Some(parent) => (
            format!("{}.{name}", parent.qualified_name),
            Some(parent.qualified_name.clone()),
        ),
        None => (name.to_string(), None),
    };
    symbols.push(ExtractedSymbol {
        name: name.to_string(),
        qualified_name,
        kind,
        visibility: None,
        start_line: line,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        byte_offset: node.start_byte() as u32,
        signature: None,
        doc_comment: None,
        scope_path,
        parent_index,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    });
    symbols.len() - 1
}

/// The innermost extractor symbol whose `[start_line, end_line]` span contains
/// `line`: the declaration the binding lives in. Value symbols are never
/// containers, so only declaration kinds qualify.
fn enclosing_symbol(line: u32, symbols: &[ExtractedSymbol]) -> Option<usize> {
    symbols
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            s.start_line <= line
                && line <= s.end_line
                && !matches!(
                    s.kind,
                    SymbolKind::Parameter
                        | SymbolKind::Variable
                        | SymbolKind::Field
                        | SymbolKind::Property
                )
        })
        .min_by_key(|(_, s)| (s.end_line - s.start_line, std::cmp::Reverse(s.start_line)))
        .map(|(i, _)| i)
}

/// Find the ref whose value is the OUTERMOST expression of `rhs` — the one whose
/// type the binding takes. Excludes refs inside functions nested STRICTLY within
/// `rhs` (callback bodies: `const x = render(() => <Page/>)` is typed by `render`,
/// not by `Page`). The selection key, minimized:
///   1. leftmost callee start — the outer expression begins at the RHS start; its
///      arguments (`render(<App/>)`, `render(p1(), p2())`) anchor to the RIGHT.
///   2. longest chain — at a shared start, the outermost expression carries the
///      most segments: `a.b().field` over the inner `a.b()`, so the binding takes
///      the field's type, not the call's return.
///   3. value-producing kind — a bare call emits both a `Calls`/`Instantiates` ref
///      AND a co-located equal-length `TypeRef`; the value ref wins.
pub(super) fn correlate_rhs_ref(
    refs: &[ExtractedRef],
    rhs: &Node,
    strategy_prefix: &str,
) -> Option<usize> {
    let r_start = rhs.start_byte() as u32;
    let r_end = rhs.end_byte() as u32;
    let nested_fn_ranges = super::flow::cfg_node_kinds_for(strategy_prefix)
        .map(|kinds| nested_function_ranges(rhs, kinds))
        .unwrap_or_default();
    let value_rank = |k: EdgeKind| -> u8 {
        match k {
            EdgeKind::Calls | EdgeKind::Instantiates => 0,
            _ => 1,
        }
    };
    refs.iter()
        .enumerate()
        .filter(|(_, r)| {
            r.byte_offset >= r_start
                && r.byte_offset < r_end
                && !nested_fn_ranges
                    .iter()
                    .any(|(s, e)| r.byte_offset >= *s && r.byte_offset < *e)
        })
        .min_by_key(|(_, r)| {
            let segments = r.chain.as_ref().map(|c| c.segments.len()).unwrap_or(0);
            (r.byte_offset, std::cmp::Reverse(segments), value_rank(r.kind))
        })
        .map(|(i, _)| i)
}

/// Byte ranges of every function node nested inside `expr`, outermost only —
/// a nested function's own nested functions are covered by its range.
pub(super) fn nested_function_ranges(
    expr: &Node,
    kinds: &crate::indexer::flow_cfg::CfgNodeKinds,
) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut c = expr.walk();
    let mut stack: Vec<Node> = expr.named_children(&mut c).collect();
    while let Some(n) = stack.pop() {
        if kinds.function_kinds.contains(&n.kind()) {
            out.push((n.start_byte() as u32, n.end_byte() as u32));
            continue;
        }
        let mut cc = n.walk();
        for ch in n.named_children(&mut cc) {
            stack.push(ch);
        }
    }
    out
}

#[cfg(test)]
#[path = "flow_bindings_tests.rs"]
mod tests;
