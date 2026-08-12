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
//
// A compiler may additionally decorate the compiled form of a namespace member
// with a fixed prefix (F#'s union-case constructors compile to `New<Case>`
// static factory methods).  When the bare target misses as a direct member,
// `LanguageProfile::compiled_name_prefixes` is probed as `<prefix><target>` —
// same namespace scan, same ambiguity rule, tried only as a fallback so a real
// bare-name declaration always wins.
// =============================================================================

use crate::indexer::resolve::engine::{LookupRule, BinderContext, LookupResult};
use crate::types::EdgeKind;

/// Outcome of scanning the prelude namespaces for one candidate name.
enum ScanOutcome {
    Found(i64),
    NotFound,
    Ambiguous,
}

/// Scans `by_name(lookup_name)` for a DIRECT member of one of `namespaces`
/// (qname exactly `<ns><sep><lookup_name>`). Dedups by qname first — multiple
/// rows for the same logical symbol (declaration merging, multi-jar stdlib)
/// count as one; two DISTINCT matching qnames is ambiguous.
fn scan(
    ctx: &BinderContext,
    namespaces: &[&str],
    separator: &str,
    edge_kind: EdgeKind,
    lookup_name: &str,
) -> ScanOutcome {
    let expected: Vec<String> = namespaces
        .iter()
        .map(|ns| format!("{ns}{separator}{lookup_name}"))
        .collect();

    let mut chosen: Option<i64> = None;
    let mut chosen_qname: Option<String> = None;
    for sym in ctx.lookup.by_name(lookup_name) {
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
            Some(_) => return ScanOutcome::Ambiguous,
        }
    }
    match chosen {
        Some(id) => ScanOutcome::Found(id),
        None => ScanOutcome::NotFound,
    }
}

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

        match scan(ctx, namespaces, separator, edge_kind, target) {
            ScanOutcome::Found(id) => {
                return LookupResult::Resolved(ctx.resolved(id, "implicit_prelude"));
            }
            ScanOutcome::Ambiguous => return LookupResult::Pass,
            ScanOutcome::NotFound => {}
        }

        for prefix in ctx.profile.compiled_name_prefixes {
            let decorated = format!("{prefix}{target}");
            match scan(ctx, namespaces, separator, edge_kind, &decorated) {
                ScanOutcome::Found(id) => {
                    return LookupResult::Resolved(ctx.resolved(id, "implicit_prelude"));
                }
                ScanOutcome::Ambiguous => return LookupResult::Pass,
                ScanOutcome::NotFound => {}
            }
        }
        LookupResult::Pass
    }
}

#[cfg(test)]
#[path = "implicit_prelude_tests.rs"]
mod tests;
