// =============================================================================
// scala/type_checker.rs — Scala type checker
//
// Scala has generics + wildcard-import namespace lookups (`import foo._`).
// Chain walking is the unified `crate::type_checker::chain::resolve_via_chain`
// with the Scala config.
// =============================================================================

use super::predicates;
use crate::indexer::resolve::engine::{FileContext, RefContext, Resolution, SymbolLookup};
use crate::type_checker::chain::{self, ChainConfig, NamespaceLookup, identity_normalize};
use crate::type_checker::TypeChecker;
use crate::types::{EdgeKind, MemberChain};

pub struct ScalaChecker;

impl ScalaChecker {
    fn chain_config() -> ChainConfig {
        ChainConfig {
            strategy_prefix: "scala",
            normalize_type: identity_normalize,
            has_self_ref: true,
            enclosing_type_kinds: &["class", "trait", "object"],
            static_type_kinds: &["class", "trait", "object", "enum", "type_alias"],
            use_generics: true,
            namespace_lookup: NamespaceLookup::WildcardOnly,
            kind_compatible: predicates::kind_compatible,
        }
    }
}

impl TypeChecker for ScalaChecker {
    fn language_id(&self) -> &str {
        "scala"
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
        let config = Self::chain_config();
        chain::resolve_via_chain(&config, chain_ref, edge_kind, file_ctx, ref_ctx, lookup)
    }
}
