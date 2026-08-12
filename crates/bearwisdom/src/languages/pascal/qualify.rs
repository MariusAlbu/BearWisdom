// =============================================================================
// languages/pascal/qualify.rs — unit-qualified top-level qnames
// =============================================================================

use crate::types::{ExtractedSymbol, SymbolKind};

/// Unit-qualify every direct child of the file's `unit`/`program` root
/// Namespace symbol (index 0): `<UnitName>.<name>`. Free functions and
/// top-level types/vars otherwise carry a bare `qualified_name` identical to
/// their `name` (every `decls.rs` emitter passes the same string for both),
/// which collapses same-name declarations from DIFFERENT units onto one
/// qname and silently first-picks between them in any qname-keyed lookup.
///
/// Skips a symbol whose `qualified_name` already contains `.` — the
/// out-of-class method-implementation shape (`procedure TFoo.Bar;`) already
/// carries a dotted name/qname and must not be re-prefixed into three
/// segments. Skips `Namespace`-kind children (the `"uses"` block's own
/// bookkeeping symbol) — nothing looks those up by qname.
///
/// `.inc` fragments have no `unit`/`program` header of their own (index 0 is
/// never a `Namespace` symbol there), so this is a no-op for them; their
/// top-level declarations stay bare and reach their spliced-in unit's scope
/// through `PascalPlugin::extra_wildcard_imports` instead (see `main_unit`).
pub(super) fn qualify_top_level_qnames(symbols: &mut [ExtractedSymbol]) {
    let Some(unit_name) = symbols
        .first()
        .filter(|s| s.kind == SymbolKind::Namespace)
        .map(|s| s.name.clone())
    else {
        return;
    };
    for sym in symbols.iter_mut().skip(1) {
        if sym.parent_index == Some(0)
            && sym.kind != SymbolKind::Namespace
            && !sym.qualified_name.contains('.')
        {
            sym.qualified_name = format!("{unit_name}.{}", sym.qualified_name);
        }
    }
}

#[cfg(test)]
#[path = "qualify_tests.rs"]
mod tests;
