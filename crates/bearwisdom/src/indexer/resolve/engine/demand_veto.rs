// =============================================================================
// engine/demand_veto — does an internal definition suppress a demand pull?
//
// An untagged ref materializes an external file only when no INTERNAL symbol
// claims the name: an internal definition outranks an external one at resolve
// time, so pulling an external homonym would add a symbol nothing can bind.
// The claim is narrower than name equality — it holds only for a definition
// the ref could actually bind:
//
//   * kind-compatible with the ref's edge kind, per the language profile's
//     `KindTable` (a METHOD named `InvalidOperationException` is not what
//     `new InvalidOperationException(...)` instantiates);
//   * written in a language the collecting file can CO-BIND — the same
//     language, or one an active ecosystem co-declares with it, the same
//     ecosystem-derived relation that gates external candidates by language.
//     A TypeScript class `DateTime` says nothing about a C# `DateTime` ref.
//
// A homonym failing either test leaves the pull open. The veto gates only what
// gets MATERIALIZED; ranking still prefers the internal candidate when the ref
// is bound, so a widened pull set cannot steal a binding from internal code.
// =============================================================================

use std::str::FromStr;

use rustc_hash::FxHashMap;

use crate::indexer::resolve::engine::compilation::Compilation;
use crate::indexer::resolve::engine::contract::SymbolLookup;
use crate::type_checker::profile::language_profile::{
    KindCompatibility, KindTable, LanguageProfile, PERMISSIVE_KIND_TABLE,
};
use crate::types::{EdgeKind, ExtractedRef, SymbolKind};

/// Parse language per indexed file path — the identity a `Symbol` does not
/// carry. Borrowed from the `ParsedFile` set the resolve pass owns.
pub type FileLanguages<'a> = FxHashMap<&'a str, &'a str>;

/// The veto test for one collecting file: its language, that language's
/// edge-kind × symbol-kind table, and the per-file language map candidates are
/// checked against.
pub struct DemandVeto<'a> {
    lang: &'a str,
    kinds: KindTable,
    file_langs: &'a FileLanguages<'a>,
}

impl<'a> DemandVeto<'a> {
    /// Build the test for a file of `lang`. A language with no profile gets the
    /// permissive table, which admits every symbol kind.
    pub fn new(
        lang: &'a str,
        profiles: &FxHashMap<&'static str, &'static LanguageProfile>,
        file_langs: &'a FileLanguages<'a>,
    ) -> Self {
        let kinds = profiles
            .get(lang)
            .map_or(PERMISSIVE_KIND_TABLE, |p| p.kind_compatible_table);
        Self {
            lang,
            kinds,
            file_langs,
        }
    }

    /// True when an internal definition of `r`'s target name is a binding
    /// candidate for `r` — a co-bound language, compatible kind — so no
    /// external file needs materializing on its account.
    pub fn vetoes(&self, tree: &Compilation, r: &ExtractedRef) -> bool {
        tree.by_name(&r.target_name).iter().any(|s| {
            !s.file_path.starts_with("ext:")
                && self.co_bound_language(tree, &s.file_path)
                && self.kind_ok(r.kind, &s.kind)
        })
    }

    /// True when `file`'s recorded language may co-bind with the collecting
    /// file's, per the compilation's ecosystem-derived language relation. A
    /// file absent from the map carries no language evidence and cannot veto.
    fn co_bound_language(&self, tree: &Compilation, file: &str) -> bool {
        match self.file_langs.get(file) {
            Some(lang) => tree.ext_langs.co_bound(self.lang, lang),
            None => false,
        }
    }

    /// Profile-table kind compatibility. An unrecognised kind string defaults
    /// permissive, matching the gate the rule ladder applies to candidates.
    fn kind_ok(&self, edge: EdgeKind, sym_kind: &str) -> bool {
        match SymbolKind::from_str(sym_kind) {
            Ok(parsed) => KindCompatibility::check(self.kinds, edge, parsed),
            Err(_) => true,
        }
    }
}

#[cfg(test)]
#[path = "demand_veto_tests.rs"]
mod tests;
