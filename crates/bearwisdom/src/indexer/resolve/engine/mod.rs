// =============================================================================
// indexer/resolve/engine — resolution engine façade
//
// The engine is the resolve loop's contract surface: it holds the symbol
// index, dispatches per-language resolvers, and exposes the small helpers
// (chain walking, npm-package extraction, scope-chain construction) that
// language resolvers and the resolve loop both consume.
//
// This file is the public API surface only — each submodule owns one
// responsibility:
//
//   * types         — public data contracts (ChainMiss, FileContext,
//                     RefContext, Resolution, SymbolInfo, TypeInfo)
//   * lookup        — SymbolLookup trait
//   * index         — SymbolIndex struct + build/augment/classify/lookup_impl
//   * chain_walker  — type-inference chain walker and its string helpers
//   * common        — `infer_external_common` external-classification helper
//   * util          — scope-chain construction, npm-package extraction,
//                     ambient-path detection, type-kind classification
// =============================================================================

pub mod chain_walker;
pub mod common;
pub mod index;
pub mod lookup;
pub mod types;
pub mod util;

pub use chain_walker::{find_member_via_inheritance, infer_external_from_chain};
pub use common::infer_external_common;
pub use index::{LocalTypeCache, SymbolIndex};
pub use lookup::SymbolLookup;
pub use types::{
    intern_yield_type, ChainMiss, FileContext, ImportEntry, RefContext, Resolution,
    SymbolInfo, TypeInfo,
};
pub use util::build_scope_chain;

// Crate-visible re-exports for items the engine's own submodules (and a
// handful of language resolvers) consult via the engine path. Keeps every
// `crate::indexer::resolve::engine::<name>` call site stable across the
// internal carve-up.
pub(crate) use chain_walker::{
    find_matching_bracket, first_generic_arg, infer_type_from_chain, merge_where_bounds,
    parse_generic_param_clause, parse_return_type_from_signature, parse_return_type_positional,
    parse_type_head_and_args, resolve_type_name_in_scope, strip_generic_args,
};
pub(crate) use util::{
    common_prefix_len, file_belongs_to_npm_package, is_ambient_global_lib_path, is_type_like_kind,
    npm_package_from_external_path, npm_package_from_specifier,
};

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
