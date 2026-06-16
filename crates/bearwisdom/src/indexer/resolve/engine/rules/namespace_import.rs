// =============================================================================
// engine/rules/namespace_import — dotted namespace import brings a whole
// namespace into scope
//
// C# `using eShop.Catalog.API.Model;` then `CatalogItem`: the import brings
// the WHOLE namespace into scope, not the type. Form `{ns}.{target}` for each
// candidate prefix derived from the import entry and look it up.
//
// `candidate_namespace_prefixes` is inlined here — it is specific to this rule.
// =============================================================================

use crate::indexer::resolve::engine::contract::ImportEntry;
use crate::indexer::resolve::engine::support::normalize_name;
use crate::indexer::resolve::engine::{LookupRule, BinderContext, LookupResult};
use crate::type_checker::profile::language_profile::NameNormalization;

pub struct NamespaceImportRule;

impl LookupRule for NamespaceImportRule {
    fn name(&self) -> &'static str {
        "namespace_import"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        let target = ctx.target();
        let edge_kind = ctx.edge_kind();
        let norm = ctx.profile.name_normalization;

        for import in &ctx.file_ctx.imports {
            for prefix in candidate_namespace_prefixes(import) {
                if !prefix.contains('.') {
                    continue;
                }
                let qname = format!("{prefix}.{target}");
                if let Some(sym) = ctx.lookup.by_qualified_name(&qname) {
                    if (ctx.kind)(edge_kind, &sym.kind) {
                        return LookupResult::Resolved(
                            ctx.resolved(sym.id, "default_namespace_import"),
                        );
                    }
                }
                // Case-folding fallback: the `{prefix}.{target}` form may differ
                // in case from the keyed qname. Scan the prefix's members and
                // accept one whose qname folds equal to the probe. Gated on a
                // non-identity spec — case-sensitive languages skip it.
                if !matches!(norm, NameNormalization::None) {
                    let expected_norm = normalize_name(norm, &qname);
                    for sym in ctx.lookup.in_namespace(prefix) {
                        if normalize_name(norm, &sym.qualified_name) == expected_norm
                            && (ctx.kind)(edge_kind, &sym.kind)
                        {
                            return LookupResult::Resolved(
                                ctx.resolved(sym.id, "default_namespace_import"),
                            );
                        }
                    }
                }
            }
        }
        LookupResult::Pass
    }
}

/// Candidate namespace prefixes for an import entry: the module path (when
/// non-empty) first, then the imported name (when non-empty and distinct).
fn candidate_namespace_prefixes(import: &ImportEntry) -> impl Iterator<Item = &str> {
    let mut prefixes: Vec<&str> = Vec::with_capacity(2);
    if let Some(m) = import.module_path.as_deref() {
        if !m.is_empty() {
            prefixes.push(m);
        }
    }
    let name = import.imported_name.as_str();
    if !name.is_empty() && !prefixes.iter().any(|&p| p == name) {
        prefixes.push(name);
    }
    prefixes.into_iter()
}

#[cfg(test)]
#[path = "namespace_import_tests.rs"]
mod tests;
