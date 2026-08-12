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
    /// Language name → code. Seeded from active-ecosystem language sets at
    /// snapshot; recording an external file interns its language on demand so
    /// candidate-side checks recognize ecosystem-less languages too.
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

    /// Record an external file's parse language. The language is interned on
    /// first sight even when no active ecosystem declares it: a CONSTRAINED
    /// receiver must be able to reject a candidate whose language is known and
    /// outside its visible set, or every ecosystem-less external language
    /// (a demand-pulled toolchain source, a stray vendored tree) leaks through
    /// the unknown-language pass in `blocked`. Receiver-side semantics are
    /// unchanged — `visible` entries come only from active ecosystems, so a
    /// language interned here gains no binding constraint of its own.
    pub(crate) fn record_file(&mut self, path: &Arc<str>, language: &str) {
        let code = self.intern(language);
        self.file_lang.entry(Arc::clone(path)).or_insert(code);
    }

    /// `record_file` for a path not yet interned as an `Arc<str>` — the DB
    /// reload path, where the first recording wins.
    pub(crate) fn record_path(&mut self, path: &str, language: &str) {
        if self.file_lang.contains_key(path) {
            return;
        }
        let code = self.intern(language);
        self.file_lang.insert(Arc::from(path), code);
    }

    /// Code for `language`, assigning the next free one on first sight.
    /// Code values are process-local and never persisted; `blocked`/`co_bound`
    /// outcomes depend only on set membership, not on assignment order.
    fn intern(&mut self, language: &str) -> u16 {
        if let Some(&code) = self.codes.get(language) {
            return code;
        }
        let code = self.codes.len() as u16;
        self.codes.insert(language.to_string(), code);
        code
    }

    /// The codes a file of `lang` may bind, or `None` when `lang` carries no
    /// visibility constraint — meaning "do not filter".
    pub(crate) fn allowed(&self, lang: &str) -> Option<&FxHashSet<u16>> {
        self.visible.get(self.codes.get(lang)?)
    }

    /// True when a declaration written in `other` is a binding candidate for a
    /// file of `lang`: the same language always, plus any language an active
    /// ecosystem co-declares with it. A pair with no recorded relation co-binds
    /// on identity alone — an absent relation is not evidence of one.
    ///
    /// The same relation `allowed` gates external candidates by, asked as a
    /// name-to-name question so a caller holding a candidate's LANGUAGE rather
    /// than its file path can consult it.
    pub(crate) fn co_bound(&self, lang: &str, other: &str) -> bool {
        if lang == other {
            return true;
        }
        let (Some(&code), Some(&other_code)) = (self.codes.get(lang), self.codes.get(other)) else {
            return false;
        };
        self.visible
            .get(&code)
            .is_some_and(|langs| langs.contains(&other_code))
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
