use std::sync::Arc;

use rustc_hash::FxHashMap;

use super::LanguagePlugin;

/// Central registry mapping language IDs to their plugin implementations.
///
/// Every language ID resolves to either a dedicated plugin or the generic
/// fallback — `get()` never returns `None`.
pub struct LanguageRegistry {
    plugins: Vec<Arc<dyn LanguagePlugin>>,
    by_lang_id: FxHashMap<String, usize>,
    generic: Arc<dyn LanguagePlugin>,
}

impl LanguageRegistry {
    /// Create a new registry with the given generic fallback plugin.
    pub fn new(generic: Arc<dyn LanguagePlugin>) -> Self {
        Self {
            plugins: Vec::new(),
            by_lang_id: FxHashMap::default(),
            generic,
        }
    }

    /// Register a language plugin. All language IDs it claims are mapped.
    pub fn register(&mut self, plugin: Arc<dyn LanguagePlugin>) {
        let idx = self.plugins.len();
        for &lang_id in plugin.language_ids() {
            self.by_lang_id.insert(lang_id.to_string(), idx);
        }
        self.plugins.push(plugin);
    }

    /// Get the plugin for a language ID. Returns the dedicated plugin if one
    /// is registered, otherwise the generic fallback. Never returns None.
    pub fn get(&self, lang_id: &str) -> &dyn LanguagePlugin {
        match self.by_lang_id.get(lang_id) {
            Some(&idx) => self.plugins[idx].as_ref(),
            None => self.generic.as_ref(),
        }
    }

    /// Get a dedicated plugin only (no fallback).
    pub fn get_dedicated(&self, lang_id: &str) -> Option<&dyn LanguagePlugin> {
        self.by_lang_id
            .get(lang_id)
            .map(|&idx| self.plugins[idx].as_ref())
    }

    /// All registered dedicated plugins (excludes the generic fallback).
    pub fn all(&self) -> &[Arc<dyn LanguagePlugin>] {
        &self.plugins
    }

    /// Get the tree-sitter grammar for a language ID.
    /// Checks the dedicated plugin first, then the generic fallback.
    pub fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        self.get(lang_id).grammar(lang_id)
    }

    /// Returns true if a dedicated (non-generic) plugin is registered for this ID.
    pub fn has_dedicated(&self, lang_id: &str) -> bool {
        self.by_lang_id.contains_key(lang_id)
    }

    /// Number of dedicated plugins registered.
    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    /// Whether the registry has no dedicated plugins.
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }
}
