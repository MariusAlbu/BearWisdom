// =============================================================================
// type_checker/core — foundation layer
//
// types.rs holds the Type / TypeId / TypeArena primitives every other layer
// consumes. symbol_types.rs maps DB symbol ids to per-symbol type metadata.
// =============================================================================

pub mod generics;
pub mod symbol_types;
pub mod types;

pub use generics::{substitute, GenericEnv};
pub use symbol_types::{SymbolIdMap, SymbolTypeData, SymbolTypeMap};
pub use types::{
    GenericParamData, GenericParamId, LitValue, PrimKind, Type, TypeArena, TypeId,
};

#[cfg(test)]
#[path = "foundation_gate_tests.rs"]
mod foundation_gate_tests;
