use std::collections::HashSet;
use std::sync::Arc;

use rustc_hash::FxHashMap;

use super::LanguagePlugin;
use crate::types::ParsedFile;

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

    /// L1: Return every language actually observed in a parsed-file set —
    /// host file language plus any embedded sub-language recorded on
    /// individual symbols.
    ///
    /// Differs from a naive `files.iter().map(|f| f.language).collect()`
    /// pass in that it also picks up languages that only appear inside
    /// embedded regions (e.g. C# inside Razor `.cshtml`, TypeScript inside
    /// Vue `<script>` blocks). A Razor-only MVC project still reports
    /// `{razor, csharp, javascript}` when the host extractor dispatched
    /// into those sub-extractors.
    pub fn detected_languages(files: &[ParsedFile]) -> HashSet<String> {
        let mut langs = HashSet::new();
        for f in files {
            langs.insert(f.language.clone());
            for origin in f.symbol_origin_languages.iter().flatten() {
                langs.insert(origin.clone());
            }
        }
        langs
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn blank_file(lang: &str, origins: Vec<Option<String>>) -> ParsedFile {
        ParsedFile {
            path: format!("f.{lang}"),
            language: lang.to_string(),
            content_hash: String::new(),
            size: 0,
            line_count: 0,
            mtime: None,
            package_id: None,
            symbols: Vec::new(),
            refs: Vec::new(),
            routes: Vec::new(),
            db_sets: Vec::new(),
            symbol_origin_languages: origins,
            ref_origin_languages: Vec::new(),
            symbol_from_snippet: Vec::new(),
            content: None,
            has_errors: false,
        }
    }

    #[test]
    fn detected_languages_host_only() {
        let files = vec![
            blank_file("csharp", vec![]),
            blank_file("typescript", vec![]),
            blank_file("csharp", vec![]),
        ];
        let langs = LanguageRegistry::detected_languages(&files);
        assert_eq!(langs, HashSet::from(["csharp".to_string(), "typescript".to_string()]));
    }

    #[test]
    fn detected_languages_includes_embedded_origins() {
        // A .cshtml file hosts razor but its symbols come from csharp + javascript.
        let files = vec![blank_file(
            "razor",
            vec![
                None,
                Some("csharp".to_string()),
                Some("javascript".to_string()),
                Some("csharp".to_string()),
            ],
        )];
        let langs = LanguageRegistry::detected_languages(&files);
        assert_eq!(
            langs,
            HashSet::from([
                "razor".to_string(),
                "csharp".to_string(),
                "javascript".to_string()
            ])
        );
    }

    #[test]
    fn detected_languages_empty_input() {
        let langs = LanguageRegistry::detected_languages(&[]);
        assert!(langs.is_empty());
    }
}
