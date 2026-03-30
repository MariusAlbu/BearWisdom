// =============================================================================
// indexer/resolve/rules/mod.rs — Language resolver registry
//
// Each language's resolution rules live in their own file. This module
// collects them into a Vec for the ResolutionEngine.
//
// To add a new language:
//   1. Create a new file (e.g., `kotlin.rs`) implementing LanguageResolver
//   2. Add `mod kotlin;` below
//   3. Add an instance to `default_resolvers()`
// =============================================================================

use super::engine::LanguageResolver;
use std::sync::Arc;

// Language rule modules (add new languages here)
mod csharp;
mod go;
mod java;
mod php;
mod python;
mod ruby;
mod rust_lang;
mod typescript;

/// Returns all built-in language resolvers.
///
/// The resolution engine calls this at construction time. Languages
/// not listed here fall back to the heuristic resolver.
pub fn default_resolvers() -> Vec<Arc<dyn LanguageResolver>> {
    vec![
        Arc::new(csharp::CSharpResolver),
        Arc::new(go::GoResolver),
        Arc::new(java::JavaResolver),
        Arc::new(php::PhpResolver),
        Arc::new(python::PythonResolver),
        Arc::new(ruby::RubyResolver),
        Arc::new(rust_lang::RustResolver),
        Arc::new(typescript::TypeScriptResolver),
    ]
}
