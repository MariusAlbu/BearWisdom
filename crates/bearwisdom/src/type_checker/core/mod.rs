// =============================================================================
// type_checker/core — foundation layer
//
// types.rs holds the Type / TypeId / TypeArena primitives the resolution engine
// and the extractors consume. The old resolver's chain/members/dispatch/
// generics/inference/pattern/supertype/symbol_types machinery was deleted with
// the legacy engine — the new engine in indexer/resolve/engine/ owns resolution.
// =============================================================================

pub mod arena_merge;
pub mod types;

pub use types::{GenericParamData, GenericParamId, LitValue, PrimKind, Type, TypeArena, TypeId};
