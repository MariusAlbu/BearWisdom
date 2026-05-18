// =============================================================================
// type_checker/core — foundation layer
//
// types.rs holds the Type / TypeId / TypeArena primitives every other layer
// consumes. symbol_types.rs maps DB symbol ids to per-symbol type metadata.
// =============================================================================

pub mod generics;
pub mod inference;
pub mod members;
pub mod supertype;
pub mod symbol_types;
pub mod types;

pub use generics::{substitute, GenericEnv};
pub use inference::{infer_expression_type, unwrap_await, unwrap_iterator};
pub use members::MembersIndex;
pub use supertype::{SupertypeGraph, SupertypeWalk};
pub use symbol_types::{SymbolIdMap, SymbolTypeData, SymbolTypeMap};
pub use types::{
    GenericParamData, GenericParamId, LitValue, PrimKind, Type, TypeArena, TypeId,
};

#[cfg(test)]
#[path = "foundation_gate_tests.rs"]
mod foundation_gate_tests;

#[cfg(test)]
#[path = "lookup_gate_tests.rs"]
mod lookup_gate_tests;
