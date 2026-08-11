// =============================================================================
// engine/rules/chain_prefix — chain call's prefix resolved via import match
//
// A chain call (`import { Sdk } from 'pkg'; Sdk.helper()`) resolves `helper`
// by matching the chain's second-to-last segment against the file's imports:
// when the segment names an imported module, the target is found by matching
// any kind-compatible by-name candidate against that module's file path or a
// path segment equal to the prefix.
// =============================================================================

use crate::indexer::resolve::engine::support::file_path_matches_module;
use crate::indexer::resolve::engine::{LookupRule, BinderContext, LookupResult};

pub struct ChainPrefixRule;

impl LookupRule for ChainPrefixRule {
    fn name(&self) -> &'static str {
        "chain_prefix"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        let target = ctx.target();
        let edge_kind = ctx.edge_kind();
        let Some(chain) = ctx.r().chain.as_ref() else {
            return LookupResult::Pass;
        };
        if chain.segments.len() < 2 {
            return LookupResult::Pass;
        }
        let prefix = chain.segments[chain.segments.len() - 2].name.as_str();

        let Some(matching_import) = ctx
            .file_ctx
            .imports
            .iter()
            .find(|imp| imp.imported_name == prefix || imp.alias.as_deref() == Some(prefix))
        else {
            return LookupResult::Pass;
        };
        let module_path = matching_import.module_path.as_deref().unwrap_or("");

        let candidates = ctx.lookup.by_name(target);

        for sym in &candidates {
            if !(ctx.kind)(edge_kind, &sym.kind) {
                continue;
            }
            if file_path_matches_module(&sym.file_path, module_path) {
                return LookupResult::Resolved(ctx.resolved(sym.id, "default_chain_prefix"));
            }
        }

        for sym in &candidates {
            if !(ctx.kind)(edge_kind, &sym.kind) {
                continue;
            }
            if sym.file_path.split('/').any(|seg| seg == prefix) {
                return LookupResult::Resolved(ctx.resolved(sym.id, "default_chain_prefix"));
            }
        }

        LookupResult::Pass
    }
}

#[cfg(test)]
#[path = "chain_prefix_tests.rs"]
mod tests;
