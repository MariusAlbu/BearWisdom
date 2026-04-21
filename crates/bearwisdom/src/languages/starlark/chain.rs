// =============================================================================
// starlark/chain.rs — Starlark / Bazel chain-aware resolution
//
// Strategy: the Starlark extractor emits dotted refs like `ctx.actions.run`
// as a flat `target_name` string (no MemberChain segments from tree-sitter).
// The synthetic `ext:bazel-builtins:ctx.bzl` file has symbols with those
// EXACT qualified names (e.g. `qualified_name = "ctx.actions.run_shell"`).
//
// So for Starlark the "chain walk" is a direct qualified-name lookup:
//   1. Split `target_name` on `.` to get segments [ctx, actions, run_shell].
//   2. Verify the root segment is a known Bazel framework root.
//   3. Reassemble the full qualified name and look it up.
//   4. If found, return a real Resolution (not external classification).
//
// For refs where the chain walker misses (uncommon 3-level+ variants not in
// the static CTX_MEMBERS table), the predicate-based fallback in resolve.rs
// still classifies them as external — no regression.
//
// We also wire the unified `resolve_via_chain` for refs that DO carry a
// MemberChain (emitted by the updated extractor for attribute-access calls).
// =============================================================================

use crate::indexer::resolve::chain_walker::{
    ChainConfig, NamespaceLookup, identity_normalize, resolve_via_chain,
};
use crate::indexer::resolve::engine::{FileContext, RefContext, Resolution, SymbolLookup};
use crate::types::{EdgeKind, MemberChain};
use super::predicates::{is_bazel_framework_chain, kind_compatible};

// ---------------------------------------------------------------------------
// ChainConfig for the unified chain walker
// ---------------------------------------------------------------------------

/// Starlark / Bazel ChainConfig for the unified `resolve_via_chain` walker.
///
/// Starlark has no `self`/`this`, no generics, no namespace imports.
/// The synthetic Bazel API symbols use `SymbolKind::Method` (stored as "method"
/// in the DB). `static_type_kinds` covers the "interface" kind used by any
/// future synthetic type-level symbols (e.g. a hypothetical `ctx_api` interface
/// in the ctx.bzl virtual file).
pub(super) static STARLARK_CHAIN_CONFIG: ChainConfig = ChainConfig {
    strategy_prefix: "starlark",
    normalize_type: identity_normalize,
    has_self_ref: false,
    enclosing_type_kinds: &[],
    static_type_kinds: &["class", "interface", "struct"],
    use_generics: false,
    namespace_lookup: NamespaceLookup::None,
    kind_compatible,
};

// ---------------------------------------------------------------------------
// Direct qualified-name lookup for Bazel framework chains
// ---------------------------------------------------------------------------

/// Resolve a Bazel framework chain ref (e.g. `ctx.actions.run_shell`) by
/// looking up the exact qualified name in the symbol index.
///
/// The synthetic `ext:bazel-builtins:ctx.bzl` ParsedFile has symbols whose
/// `qualified_name` is the dotted path verbatim. A direct lookup produces a
/// real resolved edge with strategy `"starlark_ctx_chain"` rather than
/// opaque external classification.
///
/// Returns `None` when the qualified name is not in the index, allowing the
/// predicate fallback to still classify the ref as external.
pub(super) fn resolve_ctx_chain_direct(
    target: &str,
    edge_kind: EdgeKind,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    // Only handles dotted refs whose root is a Bazel framework parameter.
    if !is_bazel_framework_chain(target) || !target.contains('.') {
        return None;
    }

    // Direct qualified-name hit against the synthetic ctx symbols.
    if let Some(sym) = lookup.by_qualified_name(target) {
        if kind_compatible(edge_kind, &sym.kind) {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 1.0,
                strategy: "starlark_ctx_chain",
                resolved_yield_type: None,
            });
        }
    }

    // Walk from segments for 2-segment refs: `ctx.actions` where `ctx.actions`
    // is directly indexed as a symbol (it is — see CTX_MEMBERS in bcr).
    // This is already handled by the by_qualified_name call above. For
    // 3-segment refs like `ctx.actions.run_shell`, same path applies.

    // No exact hit — caller falls back to predicate-external.
    None
}

// ---------------------------------------------------------------------------
// Chain resolver entry point
// ---------------------------------------------------------------------------

/// Attempt chain-walker resolution for a Starlark ref.
///
/// Two paths:
/// 1. If the ref carries an explicit `MemberChain` (≥2 segments from the
///    updated extractor), invoke the unified `resolve_via_chain`.
/// 2. If the `target_name` is a dotted framework-chain string, try the
///    direct qualified-name lookup.
///
/// Returns `None` when both paths miss, letting the caller fall through to
/// the predicate-based external classification (kept as fallback).
pub(super) fn resolve(
    chain: Option<&MemberChain>,
    target: &str,
    edge_kind: EdgeKind,
    file_ctx: Option<&FileContext>,
    ref_ctx: &RefContext,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    // Path 1: structured chain from extractor (future-proof for richer chains).
    if let Some(mc) = chain {
        if mc.segments.len() >= 2 {
            if let Some(res) =
                resolve_via_chain(&STARLARK_CHAIN_CONFIG, mc, edge_kind, file_ctx, ref_ctx, lookup)
            {
                return Some(res);
            }
        }
    }

    // Path 2: flat dotted target_name → direct qualified lookup.
    resolve_ctx_chain_direct(target, edge_kind, lookup)
}
