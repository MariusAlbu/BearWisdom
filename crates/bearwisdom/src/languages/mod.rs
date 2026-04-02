//! Language plugin system for BearWisdom.
//!
//! Each language is a self-contained plugin that provides:
//! - Tree-sitter grammar loading
//! - Scope configuration for qualified name construction
//! - Symbol + reference extraction from source code
//!
//! Resolution (turning reference names into resolved symbol edges) is provided
//! separately via [`crate::indexer::resolve::engine::LanguageResolver`] to avoid
//! circular dependencies — resolvers need the full symbol index, which isn't
//! available during extraction.
//!
//! # Adding a new language
//!
//! 1. Create `languages/<lang>/mod.rs` with a struct implementing [`LanguagePlugin`]
//! 2. Add extraction logic in `extract.rs` (and sub-files as needed)
//! 3. Optionally add a resolver in `resolve.rs` implementing `LanguageResolver`
//! 4. Register the plugin in [`default_registry()`]
//! 5. Register the resolver (if any) in [`default_resolvers()`]

pub mod registry;

use crate::parser::extractors::ExtractionResult;
use crate::parser::scope_tree::ScopeKind;

pub use registry::LanguageRegistry;

/// A language plugin provides grammar, scope config, and extraction for one or
/// more language IDs (e.g., TypeScript handles both "typescript" and "tsx").
pub trait LanguagePlugin: Send + Sync + 'static {
    /// Primary identifier (e.g., "typescript").
    fn id(&self) -> &str;

    /// All language IDs this plugin handles (e.g., `&["typescript", "tsx"]`).
    fn language_ids(&self) -> &[&str];

    /// File extensions this plugin claims (e.g., `&[".ts", ".tsx"]`).
    /// Used for documentation and validation; detection is in bearwisdom-profile.
    fn extensions(&self) -> &[&str];

    /// Get the tree-sitter grammar for a specific language ID.
    /// Returns `None` if the ID isn't handled by this plugin.
    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language>;

    /// Scope-creating node kinds for the scope tree builder.
    fn scope_kinds(&self) -> &[ScopeKind];

    /// Extract symbols and references from source code.
    ///
    /// - `source`: the file content
    /// - `file_path`: relative path (used for heuristics like `.tsx` detection)
    /// - `lang_id`: the language ID from detection (e.g., "typescript" or "tsx")
    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult;
}

// ---------------------------------------------------------------------------
// Built-in plugin registration
// ---------------------------------------------------------------------------

use std::sync::Arc;

// Language modules will be added here as they migrate:
// pub mod typescript;
// pub mod rust_lang;
// pub mod csharp;
// ...

/// Build the default language registry with all built-in plugins.
///
/// During the migration period, this coexists with the match statement in
/// `indexer/full.rs`. Once all languages are migrated, the match disappears
/// and this becomes the sole dispatch mechanism.
pub fn default_registry() -> LanguageRegistry {
    // The generic plugin handles any language with a tree-sitter grammar
    // but no dedicated extractor.
    let generic = Arc::new(GenericPlugin);
    let reg = LanguageRegistry::new(generic);

    // Dedicated plugins will be registered here as they migrate:
    // reg.register(Arc::new(typescript::TypeScriptPlugin));
    // reg.register(Arc::new(rust_lang::RustPlugin));
    // ...

    reg
}

/// Collect language-specific resolvers from all language modules.
///
/// During migration, this coexists with `indexer::resolve::rules::default_resolvers()`.
/// Once all resolvers are migrated, this replaces it.
pub fn default_resolvers() -> Vec<Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
    vec![
        // Resolvers will be registered here as they migrate:
        // Arc::new(typescript::TypeScriptResolver),
        // Arc::new(rust_lang::RustResolver),
        // ...
    ]
}

// ---------------------------------------------------------------------------
// Generic fallback plugin
// ---------------------------------------------------------------------------

/// Fallback plugin that handles any language with a tree-sitter grammar
/// but no dedicated extractor. Uses heuristic extraction based on common
/// node kinds across languages.
struct GenericPlugin;

impl LanguagePlugin for GenericPlugin {
    fn id(&self) -> &str {
        "generic"
    }

    fn language_ids(&self) -> &[&str] {
        // The generic plugin doesn't claim any specific IDs — it's the fallback.
        &[]
    }

    fn extensions(&self) -> &[&str] {
        &[]
    }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        crate::parser::languages::get_language(lang_id)
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        // The generic extractor has its own per-language scope configs.
        // Those are consulted inside `generic::extract()`.
        &[]
    }

    fn extract(&self, source: &str, _file_path: &str, lang_id: &str) -> ExtractionResult {
        match crate::parser::extractors::generic::extract(source, lang_id) {
            Some(r) => ExtractionResult::new(r.symbols, r.refs, r.has_errors),
            None => ExtractionResult::empty(),
        }
    }
}
