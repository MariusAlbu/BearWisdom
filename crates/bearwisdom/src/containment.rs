// =============================================================================
// containment.rs — qname hygiene over the parent_index chain
//
// Structural containment is now an id edge (`symbols.containing_id`), walked by
// the resolve engine through the symbol arena rather than re-derived as a
// kind-tagged frame stack. What remains here is the one pre-resolution rewrite
// that the chain still drives: correcting a value-leaf's `qualified_name` when
// the extractor built it from the scope tree (which never sees a file-scope
// package/namespace declaration as an ancestor) and dropped the outer prefix.
// =============================================================================

use crate::types::{ExtractedSymbol, SymbolKind};

/// Correct the `qualified_name` of value-holding leaf symbols (parameters and
/// local variables) whose stored qname dropped an outer prefix because the
/// pusher built it from the scope tree, which never sees a file-scope package /
/// namespace declaration as an ancestor.
///
/// Runs over symbols in ascending index order. Extractors push parents before
/// their children (DFS pre-order, `idx = symbols.len()` then push then recurse),
/// so a parent's qname is already authoritative by the time a child consults it.
///
/// A symbol is corrected only when ALL hold — keeping the rewrite surgical so it
/// touches exactly the mis-qualified params/locals and nothing else:
///   - kind is `Parameter` or `Variable` (the leaf "params/locals" that bypass
///     the package-aware builder; types/methods/fields keep the extractor's
///     qname),
///   - it has a structural `parent_index`, and
///   - its stored qname matches the *dropped-prefix signature*: it ends with a
///     separator + `name`, and the segment before that is a proper suffix of the
///     parent's qname (`App.run.repo` under parent `app.App.run`). Selector-style
///     qnames (`&:hover`, `.todo-item`) and already-consistent qnames fail the
///     signature and are left byte-identical.
///
/// When corrected, `qualified_name` becomes `parent.qualified_name <sep> name`
/// (`<sep>` inferred from the parent qname) and `scope_path` is aligned to the
/// parent's qname. Symbols that aren't corrected keep both fields untouched.
pub fn normalize_qnames_from_parents(symbols: &mut [ExtractedSymbol]) {
    for i in 0..symbols.len() {
        if !matches!(
            symbols[i].kind,
            SymbolKind::Parameter | SymbolKind::Variable
        ) {
            continue;
        }
        let Some(p) = symbols[i].parent_index else {
            continue;
        };
        if p >= symbols.len() {
            continue;
        }
        let parent_qname = symbols[p].qualified_name.clone();
        if !qname_dropped_outer_prefix(&symbols[i].qualified_name, &symbols[i].name, &parent_qname)
        {
            continue;
        }
        let sep = if parent_qname.contains("::") {
            "::"
        } else {
            "."
        };
        symbols[i].qualified_name = format!("{parent_qname}{sep}{}", symbols[i].name);
        symbols[i].scope_path = Some(parent_qname);
    }
}

/// True when `qname` is the parent's qname with some non-empty OUTER prefix
/// dropped, ending in `<sep><name>`. The shape of a mis-qualified param/local:
/// `App.run.repo` (name `repo`) under parent `app.App.run` — the inner prefix
/// `App.run` is a proper, separator-aligned suffix of `app.App.run`, so the
/// package head was lost. Returns false for selector-style qnames (no
/// `<sep><name>` tail) and for qnames already carrying the full parent prefix
/// (nothing was dropped).
fn qname_dropped_outer_prefix(qname: &str, name: &str, parent: &str) -> bool {
    if parent.is_empty() || name.is_empty() {
        return false;
    }
    // The qname must decompose as `<inner><sep><name>` for a `.` or `::`
    // separator — otherwise it isn't a composed dotted/colon qname (e.g. a CSS
    // selector whose qname equals its name).
    let inner = if let Some(stripped) = qname.strip_suffix(name) {
        if let Some(i) = stripped.strip_suffix("::") {
            i
        } else if let Some(i) = stripped.strip_suffix('.') {
            i
        } else {
            return false;
        }
    } else {
        return false;
    };
    if inner.is_empty() {
        return false;
    }
    // Already fully qualified against the parent — nothing was dropped.
    if inner == parent {
        return false;
    }
    // `inner` must be a separator-aligned proper suffix of the parent qname: the
    // parent ends with `<head><sep><inner>`, i.e. only an outer head was lost.
    parent.ends_with(&format!(".{inner}")) || parent.ends_with(&format!("::{inner}"))
}

#[cfg(test)]
#[path = "containment_tests.rs"]
mod tests;
