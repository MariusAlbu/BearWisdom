// =============================================================================
// starlark/type_checker.rs — Starlark / Bazel type checker
//
// Starlark has no `self`/`this`, no generics, no namespace imports. The
// unified-chain path uses the existing static `STARLARK_CHAIN_CONFIG`
// defined in `chain.rs` (kept there because the language's bespoke
// flat-dotted lookup also lives in chain.rs and references the same
// config). The Starlark resolver continues to call `chain::resolve(...)`
// for combined unified-chain + flat-dotted handling; this checker exposes
// the unified-chain branch through the trait so generic callers (engine
// registry, future inheritance walker) see Starlark consistently.
// =============================================================================

use super::predicates;
use crate::indexer::resolve::engine::{FileContext, RefContext, Resolution, SymbolLookup};
use crate::type_checker::chain;
use crate::type_checker::TypeChecker;
use crate::types::{EdgeKind, MemberChain};

pub struct StarlarkChecker;

impl TypeChecker for StarlarkChecker {
    fn language_id(&self) -> &str {
        "starlark"
    }

    fn kind_compatible(&self, edge_kind: EdgeKind, sym_kind: &str) -> bool {
        predicates::kind_compatible(edge_kind, sym_kind)
    }

    fn resolve_chain(
        &self,
        chain_ref: &MemberChain,
        edge_kind: EdgeKind,
        file_ctx: Option<&FileContext>,
        ref_ctx: &RefContext,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        chain::resolve_via_chain(
            &super::chain::STARLARK_CHAIN_CONFIG,
            chain_ref,
            edge_kind,
            file_ctx,
            ref_ctx,
            lookup,
        )
    }
}
