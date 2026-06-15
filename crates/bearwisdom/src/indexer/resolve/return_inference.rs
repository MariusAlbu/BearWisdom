// =============================================================================
// indexer/resolve/return_inference.rs — function return-type inference support
//
// Locate every function's `return <expr>` refs so a focused fixpoint can resolve
// them (a handful per function) and fill in the return types of un-annotated
// functions BEFORE the main resolve pass — instead of re-resolving whole files to
// harvest the same returns. This is the data half of replacing the whole-file
// return-inference re-resolve with a return-ref-only pass.
// =============================================================================

use std::collections::HashMap;

use crate::types::ParsedFile;

/// One un-annotated function's return-expression refs, located for re-resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FunctionReturns {
    /// Index into the pass's `parsed` slice of the file declaring the function.
    pub(super) file: usize,
    /// Index into `parsed[file].symbols` of the function symbol.
    pub(super) fn_idx: usize,
    /// The function's qualified name — the inference target key.
    pub(super) qname: String,
    /// Indices into `parsed[file].refs` of this function's `return <expr>` refs,
    /// ascending so the conflict-join sees a deterministic order.
    pub(super) return_refs: Vec<usize>,
}

/// Group every `return <expr>` ref under its enclosing function, keeping only
/// functions with no declared return — the ones inference can fill. A function
/// with no return-flow refs (void, or a language with no return query wired)
/// never appears: it has nothing to infer.
///
/// External files are skipped wholesale: an external function's return is read
/// from its declared signature (materialized on demand at lookup time), never
/// inferred from its body here.
pub(super) fn build_function_returns(parsed: &[ParsedFile]) -> Vec<FunctionReturns> {
    // (file, fn_idx) → its return ref indices.
    let mut by_fn: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
    for (file, pf) in parsed.iter().enumerate() {
        if pf.path.starts_with("ext:") {
            continue;
        }
        for (&ref_idx, &fn_idx) in &pf.flow.flow_return_lhs {
            by_fn.entry((file, fn_idx)).or_default().push(ref_idx);
        }
    }

    let mut out = Vec::with_capacity(by_fn.len());
    for ((file, fn_idx), mut return_refs) in by_fn {
        let Some(fn_sym) = parsed[file].symbols.get(fn_idx) else {
            continue;
        };
        if fn_sym.return_type.is_some() {
            continue;
        }
        return_refs.sort_unstable();
        out.push(FunctionReturns {
            file,
            fn_idx,
            qname: fn_sym.qualified_name.clone(),
            return_refs,
        });
    }
    // HashMap iteration order is unspecified; sort for a deterministic worklist.
    out.sort_by(|a, b| (a.file, a.fn_idx).cmp(&(b.file, b.fn_idx)));
    out
}

#[cfg(test)]
#[path = "return_inference_tests.rs"]
mod tests;
