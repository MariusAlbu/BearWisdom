// =============================================================================
// languages/plugin_defaults.rs — bodies for select `LanguagePlugin` default
// methods, factored out of `mod.rs` so the trait-declaration file stays
// close to its documented contract. Each function here backs exactly one
// default method; the behavioral doc comment lives on the trait method in
// `mod.rs`, not here — this file is implementation only.
// =============================================================================

use super::demand_filter;
use super::LanguagePlugin;
use crate::types::{ExtractedRef, ExtractedSymbol, ExtractionResult};

/// Output of [`LanguagePlugin::synthesize_symbols`]: generated symbols and
/// their own refs. `refs[*].source_symbol_index` is RELATIVE to `symbols`
/// (`0` = the first synthesized symbol); the caller rebases it onto the file's
/// symbol table when splicing, the same way embedded regions are spliced.
#[derive(Default)]
pub struct Synthesized {
    pub symbols: Vec<ExtractedSymbol>,
    pub refs: Vec<ExtractedRef>,
}

/// Default body of `LanguagePlugin::language_id_for_extension`.
///
/// Generic over `?Sized` so a trait default method can pass its own `self`
/// without the unsize coercion a `&dyn` parameter would require.
pub(super) fn language_id_for_extension<'a, P: LanguagePlugin + ?Sized>(
    plugin: &'a P,
    ext: &str,
) -> Option<&'a str> {
    if plugin.extensions().iter().any(|e| e.eq_ignore_ascii_case(ext)) {
        // Default to the first declared `language_ids` entry — that's
        // what the registry's `by_lang_id` is keyed on. Falling back to
        // `plugin.id()` alone silently routed every file to the generic
        // fallback whenever a plugin's directory name diverged from its
        // language tag. Plugins with multiple language ids — TypeScript
        // splits .ts vs .tsx, C splits .c vs .cpp — must override the
        // trait method to pick per extension; this default is for
        // single-id plugins where any member of the list is correct.
        plugin.language_ids().first().copied().or_else(|| Some(plugin.id()))
    } else {
        None
    }
}

/// Default body of `LanguagePlugin::extract_with_demand`.
pub(super) fn extract_with_demand<P: LanguagePlugin + ?Sized>(
    plugin: &P,
    source: &str,
    file_path: &str,
    lang_id: &str,
    demand: Option<&std::collections::HashSet<String>>,
) -> ExtractionResult {
    let result = plugin.extract(source, file_path, lang_id);
    match demand {
        Some(d) if !d.is_empty() => demand_filter::filter_extraction_to_demand(result, d),
        _ => result,
    }
}

/// Default body of `LanguagePlugin::extract_with_arena_and_demand`.
pub(super) fn extract_with_arena_and_demand<P: LanguagePlugin + ?Sized>(
    plugin: &P,
    source: &str,
    file_path: &str,
    lang_id: &str,
    demand: Option<&std::collections::HashSet<String>>,
    arena: &crate::type_checker::core::types::TypeArena,
) -> ExtractionResult {
    let mut result = plugin.extract_with_demand(source, file_path, lang_id, demand);
    super::common::populate_return_type_ids(&mut result, arena, lang_id);
    result
}
