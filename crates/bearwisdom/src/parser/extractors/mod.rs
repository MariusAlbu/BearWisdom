//! Legacy extractors module — retained for backward compatibility.
//!
//! All language extractors have moved to `crate::languages::<lang>/`.
//! This module re-exports shared types that external code may reference.

// Re-export types now defined in crate::types
pub use crate::types::ExtractionResult;

// Re-export shared utility now in crate::languages
pub use crate::languages::emit_chain_type_ref;
