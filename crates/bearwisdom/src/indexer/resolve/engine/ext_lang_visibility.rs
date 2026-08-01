// =============================================================================
// engine/ext_lang_visibility — which languages' externals a file may bind
//
// A `Symbol` carries no language, so a bare-name lookup would rank external
// declarations from every indexed ecosystem together — a Python annotation
// `field: str` binding Rust's `str`. The rule is ecosystem-derived, not
// hardcoded: two languages co-declared by ONE active ecosystem legitimately
// cross-resolve (npm's `.d.ts` surface types JS/Vue/Svelte refs; Maven's Java
// sources type Kotlin refs); languages sharing no active ecosystem do not.
//
// Languages are interned to `u16` codes, so the per-candidate check is an
// integer set lookup rather than a string compare.
// =============================================================================

use std::sync::Arc;

use rustc_hash::{FxHashMap, FxHashSet};

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::contract::{Symbol, SymbolSet};

/// Interned per-language visibility over EXTERNAL declarations. Empty when no
/// project context was snapshot, which disables the check entirely.
#[derive(Default)]
pub(crate) struct ExtLangVisibility {
    /// Language name → code. Only languages served by an active ecosystem get
    /// one; everything else stays un-coded and therefore unfiltered.
    codes: FxHashMap<String, u16>,
    /// Receiver-language code → the candidate-language codes it may bind.
    visible: FxHashMap<u16, FxHashSet<u16>>,
    /// Parse language per EXTERNAL file, keyed by the same `Arc<str>` path the
    /// file's `Symbol` rows carry.
    file_lang: FxHashMap<Arc<str>, u16>,
}

impl ExtLangVisibility {
    /// Snapshot the active ecosystems' language sets. Codes are assigned from
    /// the sorted name set, so assignment is deterministic per build.
    pub(crate) fn snapshot(ctx: &ProjectContext) -> Self {
        let mut out = Self::default();
        let vis = ctx.ext_language_visibility();
        let mut names: Vec<&str> = vis.keys().copied().collect();
        names.sort_unstable();
        for (i, name) in names.iter().enumerate() {
            out.codes.insert((*name).to_string(), i as u16);
        }
        for (lang, langs) in &vis {
            let Some(&code) = out.codes.get(*lang) else {
                continue;
            };
            let codes: FxHashSet<u16> = langs
                .iter()
                .filter_map(|m| out.codes.get(*m).copied())
                .collect();
            out.visible.insert(code, codes);
        }
        out
    }

    /// Record an external file's parse language. A language with no code stays
    /// unrecorded, so its symbols are never filtered.
    pub(crate) fn record_file(&mut self, path: &Arc<str>, language: &str) {
        if let Some(&code) = self.codes.get(language) {
            self.file_lang.entry(Arc::clone(path)).or_insert(code);
        }
    }

    /// `record_file` for a path not yet interned as an `Arc<str>` — the DB
    /// reload path, where the first recording wins.
    pub(crate) fn record_path(&mut self, path: &str, language: &str) {
        if self.file_lang.contains_key(path) {
            return;
        }
        if let Some(&code) = self.codes.get(language) {
            self.file_lang.insert(Arc::from(path), code);
        }
    }

    /// The codes a file of `lang` may bind, or `None` when `lang` carries no
    /// visibility constraint — meaning "do not filter".
    pub(crate) fn allowed(&self, lang: &str) -> Option<&FxHashSet<u16>> {
        self.visible.get(self.codes.get(lang)?)
    }

    /// Drop external candidates whose file language is known and not allowed.
    /// Internal candidates and external files with no recorded language always
    /// pass; the borrowed set is returned untouched when nothing is dropped.
    pub(crate) fn filter<'a>(
        &self,
        set: SymbolSet<'a>,
        allowed: Option<&FxHashSet<u16>>,
    ) -> SymbolSet<'a> {
        let Some(allowed) = allowed else { return set };
        if !set.iter().any(|s| self.blocked(s, allowed)) {
            return set;
        }
        SymbolSet::Owned(
            set.into_iter()
                .filter(|s| !self.blocked(s, allowed))
                .collect(),
        )
    }

    /// True when `sym` lives in an external file whose recorded parse language
    /// is outside `allowed`.
    fn blocked(&self, sym: &Symbol, allowed: &FxHashSet<u16>) -> bool {
        if !sym.file_path.starts_with("ext:") {
            return false;
        }
        match self.file_lang.get(&*sym.file_path) {
            Some(code) => !allowed.contains(code),
            None => false,
        }
    }
}
