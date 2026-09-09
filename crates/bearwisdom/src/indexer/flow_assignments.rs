//! Assignment-query ingestion, extracted without changing legacy behavior.
use super::flow::{cached_query, FlowConfig};
use super::flow_bindings::{
    binding_symbol, correlate_lhs_symbol, correlate_rhs_ref, BindingSymbols,
};
use crate::types::{ExtractedRef, ExtractedSymbol, FlowMeta, SymbolKind};
use tree_sitter::{Node, QueryCursor, StreamingIterator};

#[cfg(test)]
#[path = "flow_assignments_tests.rs"]
mod tests;

pub(super) fn run_assignment_query(
    root: &Node,
    src: &[u8],
    cfg: &FlowConfig,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &[ExtractedRef],
    meta: &mut FlowMeta,
    bindings: BindingSymbols,
) {
    let Some(query) = cached_query(&root.language(), cfg.assignment_query) else {
        return;
    };
    let Some(lhs_cap) = query.capture_index_for_name("lhs") else {
        return;
    };
    // `@lhs.param` is a binding declared by a parameter list rather than an
    // assignment: correlated on its own line and synthesized as a `Parameter`.
    let param_cap = query.capture_index_for_name("lhs.param");
    // `@rhs` (forward inference from the initializer) and `@type` (explicit
    // annotation) are both optional — a query may capture either or both. A
    // binding with only `@type` (`let x: T;`) seeds a declared type with no
    // ref to resolve; one with only `@rhs` (`let x = expr`) drives forward
    // inference as before.
    let rhs_cap = query.capture_index_for_name("rhs");
    let type_cap = query.capture_index_for_name("type");
    // `@rhs_unwrap` is `@rhs` whose initializer applies a fallible-unwrap
    // operator (Rust `?`). Correlated to a ref like `@rhs`, but the binding is
    // additionally flagged so the resolver peels one wrapper layer.
    let unwrap_cap = query.capture_index_for_name("rhs_unwrap");
    // Object-destructure bindings: `@destruct.bind` is the bound identifier,
    // `@destruct.key` the source field name (absent for shorthand `{ a }`, where
    // the field equals the bound name). Languages whose query omits these
    // captures get `None` here and the destructure branch never fires.
    let bind_cap = query.capture_index_for_name("destruct.bind");
    let key_cap = query.capture_index_for_name("destruct.key");

    let mut cursor = QueryCursor::new();
    let mut it = cursor.matches(&*query, *root, src);
    while let Some(m) = it.next() {
        let mut lhs_node: Option<Node> = None;
        let mut lhs_is_param = false;
        let mut rhs_node: Option<Node> = None;
        let mut type_node: Option<Node> = None;
        let mut unwrap_node: Option<Node> = None;
        let mut bind_node: Option<Node> = None;
        let mut key_node: Option<Node> = None;
        for cap in m.captures {
            if cap.index == lhs_cap {
                lhs_node = Some(cap.node);
            } else if Some(cap.index) == param_cap {
                lhs_node = Some(cap.node);
                lhs_is_param = true;
            } else if Some(cap.index) == rhs_cap {
                rhs_node = Some(cap.node);
            } else if Some(cap.index) == type_cap {
                type_node = Some(cap.node);
            } else if Some(cap.index) == unwrap_cap {
                unwrap_node = Some(cap.node);
            } else if Some(cap.index) == bind_cap {
                bind_node = Some(cap.node);
            } else if Some(cap.index) == key_cap {
                key_node = Some(cap.node);
            }
        }

        // Object-destructure binding: `const { a, b: c } = f()`. Each match carries
        // one bound identifier; type it from the FIELD on the RHS's yield type
        // (`R["a"]`), recorded against the RHS ref for the seeding loop to resolve.
        if let Some(bind) = bind_node {
            if let (Ok(bind_name), Some(rhs)) = (bind.utf8_text(src), rhs_node) {
                let slot = match &meta.lexical {
                    Some(graph) => graph.symbol_at(bind.start_byte() as u32, bind_name),
                    None => {
                        correlate_lhs_symbol(bind_name, bind.start_position().row as u32, symbols)
                    }
                };
                if let Some(lhs_idx) = slot {
                    // Shorthand `{ a }` → field == bound name; `{ b: c }` → `@destruct.key`.
                    let field_key = key_node
                        .and_then(|k| k.utf8_text(src).ok())
                        .unwrap_or(bind_name)
                        .to_string();
                    if let Some(ref_idx) = correlate_rhs_ref(refs, &rhs, cfg.strategy_prefix) {
                        meta.flow_binding_destructure
                            .entry(ref_idx)
                            .or_default()
                            .push((lhs_idx, field_key));
                        if rhs.kind() == "await_expression" {
                            meta.flow_binding_destructure_await.insert(ref_idx);
                        }
                    }
                }
            }
            continue;
        }
        let Some(lhs) = lhs_node else { continue };
        let lhs_name = match lhs.utf8_text(src) {
            Ok(t) => t,
            Err(_) => continue,
        };

        // The binding's symbol: the extractor's own, or one synthesized for a
        // binding the extractor never emitted.
        let kind = if lhs_is_param {
            SymbolKind::Parameter
        } else {
            SymbolKind::Variable
        };
        let slot = match &meta.lexical {
            Some(graph) => graph.symbol_at(lhs.start_byte() as u32, lhs_name),
            None => binding_symbol(lhs_name, &lhs, kind, symbols, bindings),
        };
        let Some(lhs_idx) = slot else {
            continue;
        };

        // Explicit annotation: record the declared type text verbatim (trimmed
        // only). `intern_type_str` decomposes `Vec<Item>` into `Apply(Vec,
        // [Item])` itself, so the head still keys the member lookup AND the
        // generic argument survives for element-type projection (`v[0]`).
        if let Some(ty) = type_node {
            if let Ok(text) = ty.utf8_text(src) {
                let text = text.trim();
                if !text.is_empty() {
                    meta.flow_binding_decl_type
                        .insert(lhs_idx, text.to_string());
                }
            }
        }

        // Initializer: correlate RHS byte range → ref_idx. The ref whose
        // byte_offset is within [rhs.start_byte, rhs.end_byte) AND whose chain
        // covers the RHS's final segment wins. For a chain `foo.bar()` the
        // refs emitter creates a Calls ref at the call node — match the ref
        // with byte_offset in range AND latest (furthest-right) start. A
        // `@rhs_unwrap` capture (fallible `?`) is correlated the same way and
        // additionally flags the binding for wrapper peeling.
        let (rhs, is_unwrap) = match (rhs_node, unwrap_node) {
            (Some(r), _) => (Some(r), false),
            (None, Some(u)) => (Some(u), true),
            (None, None) => (None, false),
        };
        if let Some(rhs) = rhs {
            let ref_idx = correlate_rhs_ref(refs, &rhs, cfg.strategy_prefix);
            if let Some(ref_idx) = ref_idx {
                meta.flow_binding_lhs.insert(ref_idx, lhs_idx);
                if is_unwrap {
                    meta.flow_binding_unwrap.insert(lhs_idx);
                }
                if rhs.kind() == "await_expression" {
                    meta.flow_binding_await.insert(lhs_idx);
                }
            } else if !meta.flow_binding_decl_type.contains_key(&lhs_idx) {
                // No resolvable RHS ref and no annotation: classify the literal
                // node kind against the language's wrapper-type table. Skips
                // bindings that already have a declared type from the `@type`
                // capture above so the annotation always wins.
                if let Some((_, wrapper)) = cfg
                    .literal_type_kinds
                    .iter()
                    .find(|(k, _)| *k == rhs.kind())
                {
                    meta.flow_binding_decl_type
                        .insert(lhs_idx, (*wrapper).to_string());
                }
            }
        }
    }
}
