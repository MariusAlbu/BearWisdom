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
pub mod lookup;
pub mod symbol_set;
pub mod types;
pub mod util;

pub use lookup::SymbolLookup;
pub use symbol_set::SymbolSet;
pub use types::{
    FileContext, ImportEntry, RefContext, SymbolInfo, Symbol, TypeInfo,
    RESOLVED_CONFIDENCE,
};

/// Backward-compatible alias: the old engine used `Resolution`; the engine
/// contract uses `SymbolInfo` for the same data shape.
pub type Resolution = SymbolInfo;
pub use util::{build_scope_chain, camel_to_kebab, lexical_normalize};

pub(crate) use chain_walker::{
    find_matching_bracket, first_generic_arg, is_jvm_language,
    is_plain_type_name, merge_where_bounds, parse_declared_type_from_signature_for_lang,
    parse_generic_param_clause, parse_param_types_from_signature,
    parse_return_type_from_jvm_descriptor, parse_return_type_from_signature,
    parse_return_type_positional, parse_return_type_trailing, parse_type_head_and_args,
    parse_type_head_and_args_bracket, resolve_type_name_in_scope, strip_generic_args,
};
pub(crate) use util::{
    common_prefix_len, file_belongs_to_npm_package, is_type_like_kind,
    npm_package_from_external_path, npm_package_from_specifier,
};
