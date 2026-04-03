//! Legacy resolver registry — all resolvers have moved to `crate::languages::<lang>/`.
//!
//! This module is retained for backward compatibility with `ResolutionEngine::new()`.
//! It returns an empty vec; the engine sources resolvers from `crate::languages::default_resolvers()`.

use super::engine::LanguageResolver;
use std::sync::Arc;

pub fn default_resolvers() -> Vec<Arc<dyn LanguageResolver>> {
    vec![]
}
