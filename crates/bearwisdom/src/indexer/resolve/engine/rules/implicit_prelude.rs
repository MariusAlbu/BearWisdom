// =============================================================================
// engine/rules/implicit_prelude — compiler-implicit namespace imports
//
// Some languages make a fixed set of namespaces available without an explicit
// import. The set is `LanguageProfile::implicit_prelude_namespaces` — per-language
// data, not code.
//
// Binds only to a DIRECT member of one of those namespaces — the candidate's
// qname must be exactly `<ns><sep><target>`.  Nested types, methods, and
// sub-namespaces carry an extra separator and are deliberately excluded, because
// the compiler demands an explicit import to reach them.
//
// Declines on ambiguity: two candidates with distinct qnames both matching a
// prelude namespace means the ref could name either one — stay unresolved rather
// than guess.  Deduplication by qname is applied first so multiple rows for the
// same logical symbol (declaration merging, multi-jar stdlib) count as one.
// =============================================================================

use crate::indexer::resolve::engine::{LookupRule, BinderContext, LookupResult};

pub struct ImplicitPreludeRule;

impl LookupRule for ImplicitPreludeRule {
    fn name(&self) -> &'static str {
        "implicit_prelude"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        let namespaces = ctx.profile.implicit_prelude_namespaces;
        if namespaces.is_empty() {
            return LookupResult::Pass;
        }
        let target = ctx.target();
        let separator = ctx.profile.qname_separator;
        if target.is_empty() || target.contains(separator) {
            return LookupResult::Pass;
        }
        let edge_kind = ctx.edge_kind();

        // Build the set of expected direct-member qnames once.
        let expected: Vec<String> = namespaces
            .iter()
            .map(|ns| format!("{ns}{separator}{target}"))
            .collect();

        // Scan by_name; dedup by qname (multi-jar stdlib duplication is NOT
        // ambiguity).  Two DISTINCT member qnames both matching → decline.
        let mut chosen: Option<i64> = None;
        let mut chosen_qname: Option<String> = None;
        for sym in ctx.lookup.by_name(target) {
            if !(ctx.kind)(edge_kind, &sym.kind) {
                continue;
            }
            if !expected.iter().any(|e| *e == sym.qualified_name) {
                continue;
            }
            match chosen_qname.as_deref() {
                None => {
                    chosen = Some(sym.id);
                    chosen_qname = Some(sym.qualified_name.clone());
                }
                Some(q) if q == sym.qualified_name => {}
                Some(_) => return LookupResult::Pass,
            }
        }
        match chosen {
            Some(id) => LookupResult::Resolved(ctx.resolved(id, "implicit_prelude")),
            None => LookupResult::Pass,
        }
    }
}

#[cfg(test)]
#[path = "implicit_prelude_tests.rs"]
mod tests;
