// =============================================================================
// languages/demand_filter.rs  —  generic post-extraction demand pruning
//
// Language-agnostic counterpart to the AST-aware demand path TypeScript runs.
// Plugins without their own demand extractor full-extract a pulled external
// file, then call `filter_extraction_to_demand` to drop every declaration the
// project never reaches.
//
// Keep-set algorithm (three layers, all run before any index remap):
//
//   1. Seeds — symbols whose leaf `name` is in the demand set.
//   2. Descendant closure — every symbol whose transitive parent chain reaches
//      a seed (fields/methods of a demanded struct must survive so `x.field`
//      resolves).
//   3. Intra-file type-ref closure — fixpoint over type-relevant refs whose
//      source is in the keep set: if the ref's target_name matches a symbol
//      defined in THIS same file, that symbol (and its descendants) enters the
//      keep set and the fixpoint repeats. This handles the case where a pulled
//      external file is never re-pulled (it enters `already_walked` after the
//      first parse), so sibling types that are referenced from a kept symbol
//      must be retained in this single pass rather than on a future iteration.
//
// Cross-file refs still drive the multi-iteration expand loop: a kept symbol's
// surviving refs feed the next iteration's chain misses, which pull the
// relevant file with its own demand set.
// =============================================================================

use crate::types::{EdgeKind, ExtractionResult};
use std::collections::HashSet;

/// Type-relevant ref kinds whose targets enter the intra-file closure.
/// These are the edge kinds the chain walker resolves through — keeping a
/// symbol's type-referenced sibling prevents a permanent unresolved ref when
/// that sibling lives in the same file.
fn is_type_dep(kind: EdgeKind) -> bool {
    matches!(
        kind,
        EdgeKind::TypeRef | EdgeKind::Inherits | EdgeKind::Implements | EdgeKind::Instantiates
    )
}

/// Prune `result` to the keep set (seeds + descendants + intra-file type-ref
/// closure), then remap every index that pointed into the pre-filter symbols
/// vec (`parent_index`, ref `source_symbol_index`, route/db-set indices).
///
/// Leaf `name` is the match key for seeds and the intra-file closure.
/// Extraction runs before any `ext:`-prefix post-processing, so symbol names
/// are bare leaves (e.g. `HANDLE`), which matches the demand-set shape.
///
/// Callers gate this on a non-empty demand set; callers must not pass an empty
/// set (the trait default short-circuits before calling here).
pub fn filter_extraction_to_demand(
    result: ExtractionResult,
    demand: &HashSet<String>,
) -> ExtractionResult {
    // Empty demand means "keep everything" — no seed can match, so without this
    // guard the closure would prune the whole file to nothing.
    if demand.is_empty() {
        return result;
    }
    let symbol_count = result.symbols.len();

    // ── Layer 0: children adjacency (parent old-index → child old-indices) ──
    // Built once; lets the descendant walk run without O(N²) rescanning.
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); symbol_count];
    for (idx, sym) in result.symbols.iter().enumerate() {
        if let Some(p) = sym.parent_index {
            if p < symbol_count {
                children[p].push(idx);
            }
        }
    }

    // ── Layer 1: seed + descendant closure ──
    let mut keep: Vec<bool> = vec![false; symbol_count];
    let mut stack: Vec<usize> = Vec::new();

    for (idx, sym) in result.symbols.iter().enumerate() {
        if demand.contains(&sym.name) && !keep[idx] {
            keep[idx] = true;
            stack.push(idx);
        }
    }
    expand_descendants(&mut keep, &mut stack, &children);

    // ── Layer 2: intra-file type-ref closure (fixpoint) ──
    // Build a name → [old-indices] map once so each fixpoint iteration is O(R)
    // where R is the number of refs, not O(R × S). Owned keys avoid a borrow
    // on `result.symbols` that would conflict with the later destructure move.
    let mut name_to_indices: std::collections::HashMap<String, Vec<usize>> =
        std::collections::HashMap::new();
    for (idx, sym) in result.symbols.iter().enumerate() {
        name_to_indices
            .entry(sym.name.clone())
            .or_default()
            .push(idx);
    }

    // Collect the type-dep refs into an owned vec so we can drop the shared
    // borrow on `result.refs` before the destructure that moves `result`.
    let type_dep_refs: Vec<(usize, String)> = result
        .refs
        .iter()
        .filter(|r| is_type_dep(r.kind))
        .map(|r| (r.source_symbol_index, r.target_name.clone()))
        .collect();

    // Fixpoint: scan type-dep refs from currently-kept symbols; when the target
    // names a same-file symbol, pull that symbol + its descendants into the keep
    // set. Repeat until a full pass adds nothing new.
    loop {
        let mut added = false;
        for (src, target) in &type_dep_refs {
            let src = *src;
            if src >= symbol_count || !keep[src] {
                continue;
            }
            if let Some(targets) = name_to_indices.get(target.as_str()) {
                for &t in targets {
                    if !keep[t] {
                        keep[t] = true;
                        stack.push(t);
                        added = true;
                    }
                }
            }
        }
        if added {
            // Expand descendants for anything newly added this pass.
            expand_descendants(&mut keep, &mut stack, &children);
        } else {
            break;
        }
    }

    // ── Remap: old-index → new-index for kept symbols ──
    let mut old_to_new: Vec<Option<usize>> = vec![None; symbol_count];
    let mut next_new = 0usize;
    for (idx, kept) in keep.iter().enumerate() {
        if *kept {
            old_to_new[idx] = Some(next_new);
            next_new += 1;
        }
    }

    let ExtractionResult {
        symbols,
        refs,
        routes,
        db_sets,
        has_errors,
        demand_contributions,
        alias_targets,
        declared_modules,
    } = result;

    // Surviving qualified names for alias_targets pruning (keyed by qname, not
    // by index, so no index remap needed — just name-set membership check).
    let kept_qnames: HashSet<String> = symbols
        .iter()
        .enumerate()
        .filter(|(idx, _)| keep[*idx])
        .map(|(_, sym)| sym.qualified_name.clone())
        .collect();

    // Symbols: preserve extraction order, remap parent_index.
    // A seed's own parent is usually NOT demanded (the enclosing module), so
    // `and_then(...flatten())` drops the link and the seed becomes a root.
    let new_symbols = symbols
        .into_iter()
        .enumerate()
        .filter(|(idx, _)| keep[*idx])
        .map(|(_, mut sym)| {
            sym.parent_index =
                sym.parent_index.and_then(|p| old_to_new.get(p).copied().flatten());
            sym
        })
        .collect();

    // Refs: keep those originating from a surviving symbol, remap source index.
    // Both type-dep and non-type-dep refs of kept symbols survive (e.g. Calls
    // refs feed the call-hierarchy graph; only the source must be kept).
    let new_refs = refs
        .into_iter()
        .filter_map(|mut r| {
            let new_idx = old_to_new.get(r.source_symbol_index).copied().flatten()?;
            r.source_symbol_index = new_idx;
            Some(r)
        })
        .collect();

    // Routes / db_sets carry a symbol index; drop if handler/property dropped,
    // remap otherwise. Dangling indices here would silently mis-attribute.
    let new_routes = routes
        .into_iter()
        .filter_map(|mut route| {
            let new_idx = old_to_new
                .get(route.handler_symbol_index)
                .copied()
                .flatten()?;
            route.handler_symbol_index = new_idx;
            Some(route)
        })
        .collect();

    let new_db_sets = db_sets
        .into_iter()
        .filter_map(|mut ds| {
            let new_idx = old_to_new
                .get(ds.property_symbol_index)
                .copied()
                .flatten()?;
            ds.property_symbol_index = new_idx;
            Some(ds)
        })
        .collect();

    let new_alias_targets = alias_targets
        .into_iter()
        .filter(|(qname, _)| kept_qnames.contains(qname))
        .collect();

    ExtractionResult {
        symbols: new_symbols,
        refs: new_refs,
        routes: new_routes,
        db_sets: new_db_sets,
        has_errors,
        // demand_contributions carries no symbol index — preserve verbatim.
        demand_contributions,
        alias_targets: new_alias_targets,
        // Declared ambient-module names carry no symbol index — preserve
        // verbatim so a demanded declaration file keeps its module keys.
        declared_modules,
    }
}

/// Drain `stack`, marking every descendant of each entry as kept.
/// Reused by both the initial seed pass and each fixpoint iteration.
fn expand_descendants(keep: &mut Vec<bool>, stack: &mut Vec<usize>, children: &[Vec<usize>]) {
    while let Some(idx) = stack.pop() {
        for &child in &children[idx] {
            if !keep[child] {
                keep[child] = true;
                stack.push(child);
            }
        }
    }
}

#[cfg(test)]
#[path = "demand_filter_tests.rs"]
mod tests;
