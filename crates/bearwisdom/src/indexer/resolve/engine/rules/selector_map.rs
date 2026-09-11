// =============================================================================
// engine/rules/selector_map — selector key → class qname lookup
//
// Gated on `ctx.profile.selector_resolution` (None → Pass). When present, the
// owning language produces normalized selector candidates for the ref target;
// this rule probes them against the index's selector→qname map.
//
// After a selector-map hit the class symbol is located either by direct qname
// lookup or by a by-name scan pinning the exact qname — the export-wrapper
// shape means the qname-keyed map may not hold the same value as `by_qualified_name`.
// =============================================================================

use crate::indexer::resolve::engine::support::index_qname_leaf;
use crate::indexer::resolve::engine::{BinderContext, LookupResult, LookupRule};
use crate::type_checker::profile::language_profile::SelectorResolution;

pub struct SelectorMapRule;

impl LookupRule for SelectorMapRule {
    fn name(&self) -> &'static str {
        "selector_map"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        let Some(cfg) = ctx.profile.selector_resolution.as_ref() else {
            return LookupResult::Pass;
        };
        apply_selector_map(ctx, cfg)
    }
}

fn apply_selector_map(ctx: &BinderContext<'_>, cfg: &SelectorResolution) -> LookupResult {
    let edge_kind = ctx.edge_kind();
    if !cfg.edge_kinds.contains(&edge_kind) {
        return LookupResult::Pass;
    }
    let target = ctx.target();
    if target.is_empty() {
        return LookupResult::Pass;
    }
    for candidate in (cfg.selector_candidates)(target) {
        let Some(class_qname) = ctx.lookup.selector_qname(&candidate) else {
            continue;
        };
        let class_qname = class_qname.to_string();
        if let Some(sym) = ctx.lookup.by_qualified_name(&class_qname) {
            if (ctx.kind)(edge_kind, &sym.kind) {
                return LookupResult::Resolved(ctx.resolved(sym.id, "default_selector_map"));
            }
        }
        // Export-wrapper qnames: fall back to a by-name scan that pins the
        // exact qname.
        let short = index_qname_leaf(&class_qname);
        for sym in ctx.lookup.by_name(short) {
            if sym.qualified_name == class_qname && (ctx.kind)(edge_kind, &sym.kind) {
                return LookupResult::Resolved(ctx.resolved(sym.id, "default_selector_map"));
            }
        }
    }
    LookupResult::Pass
}

#[cfg(test)]
#[path = "selector_map_tests.rs"]
mod tests;
