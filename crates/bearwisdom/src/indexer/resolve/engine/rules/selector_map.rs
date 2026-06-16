// =============================================================================
// engine/rules/selector_map — CSS/Angular selector → class qname lookup
//
// Gated on `ctx.profile.selector_resolution` (None → Pass). When present, the
// ref's target and each name-transformed form of it are probed against the
// index's selector→qname map. The raw target is always tried first; the
// `name_transforms` list (e.g. PascalToKebab for Angular) produces additional
// candidates in declaration order.
//
// After a selector-map hit the class symbol is located either by direct qname
// lookup or by a by-name scan pinning the exact qname — the export-wrapper
// shape means the qname-keyed map may not hold the same value as `by_qualified_name`.
// =============================================================================

use std::borrow::Cow;

use crate::indexer::resolve::engine::{LookupRule, BinderContext, LookupResult};
use crate::type_checker::profile::language_profile::{NameTransform, SelectorResolution};

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
    let mut candidates: Vec<Cow<'_, str>> = vec![Cow::Borrowed(target)];
    for transform in cfg.name_transforms {
        candidates.push(apply_name_transform(*transform, target));
    }
    for candidate in &candidates {
        let Some(class_qname) = ctx.lookup.selector_qname(candidate) else {
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
        let short = class_qname.rsplit('.').next().unwrap_or(&class_qname);
        for sym in ctx.lookup.by_name(short) {
            if sym.qualified_name == class_qname && (ctx.kind)(edge_kind, &sym.kind) {
                return LookupResult::Resolved(ctx.resolved(sym.id, "default_selector_map"));
            }
        }
    }
    LookupResult::Pass
}

/// Apply a `NameTransform` to a ref target, yielding one selector-key candidate.
fn apply_name_transform(transform: NameTransform, name: &str) -> Cow<'_, str> {
    match transform {
        NameTransform::PascalToKebab => pascal_to_kebab(name),
    }
}

/// `AppUserCard` → `app-user-card`: insert `-` at each interior uppercase
/// boundary and lowercase. A single-segment input with no interior uppercase
/// boundary is returned borrowed unchanged.
fn pascal_to_kebab(name: &str) -> Cow<'_, str> {
    let needs_split = name
        .char_indices()
        .any(|(i, c)| i > 0 && c.is_ascii_uppercase());
    if !needs_split && name.chars().all(|c| !c.is_ascii_uppercase()) {
        return Cow::Borrowed(name);
    }
    let mut out = String::with_capacity(name.len() + 4);
    for (i, ch) in name.chars().enumerate() {
        if ch.is_ascii_uppercase() && i > 0 {
            out.push('-');
        }
        out.extend(ch.to_lowercase());
    }
    Cow::Owned(out)
}

#[cfg(test)]
#[path = "selector_map_tests.rs"]
mod tests;
