// =============================================================================
// engine/contract — the new engine's own resolution contract
//
// An owned copy of the resolution vocabulary: the data types, the SymbolLookup
// trait, SymbolSet, and the stateless type/scope parse helpers. Duplicated from
// the frozen legacy engine so the new island depends on nothing under legacy/.
// The two are independent — deleting legacy/ ripples into nothing here. These
// types are renamed to the Roslyn vocabulary in a following pass.
// =============================================================================

pub mod chain_walker;
pub mod flow_cache;
pub mod generic_return;
pub mod include_lookup;
pub mod lookup;
mod lookup_display;
mod lookup_nominal;
pub mod member_applicability;
pub mod symbol_set;
pub mod types;
pub mod util;

pub use flow_cache::FlowCacheLookup;
pub use include_lookup::IncludeLookup;
pub use lookup::SymbolLookup;
pub use symbol_set::SymbolSet;
pub use types::{
    FileContext, ImportEntry, RefContext, Symbol, SymbolInfo, TypeInfo, RESOLVED_CONFIDENCE,
};

/// Backward-compatible alias: the old engine used `Resolution`; the engine
/// contract uses `SymbolInfo` for the same data shape.
pub type Resolution = SymbolInfo;
pub use util::{build_scope_chain, camel_to_kebab, lexical_normalize};

pub(crate) use chain_walker::resolve_type_name_in_scope;
pub(crate) use util::{common_prefix_len, is_type_like_kind};
