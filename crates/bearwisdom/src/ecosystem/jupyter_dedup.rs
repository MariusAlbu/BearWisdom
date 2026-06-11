// =============================================================================
// ecosystem/jupyter_dedup.rs — translated-notebook deduplication
//
// Localized Jupyter courseware ships the same notebooks once per locale under a
// `translations/<locale>/<canonical-rel-path>/X.ipynb` tree. The canonical
// notebook lives at `<canonical-rel-path>/X.ipynb` (outside `translations/`);
// every entry under `translations/<locale>/` is the same notebook in another
// language. Counting all of them inflates the corpus by the locale count.
//
// The locale-copy is classified `origin='external'` at index time so only the
// canonical notebook counts toward the project's resolution rate — the repo
// layout itself is the dedup signal, no content hashing required.
//
// This module supplies the classification predicate consumed during origin
// assignment in `indexer/full.rs`.
// =============================================================================

/// True when `rel_path` (project-root-relative) is a per-locale copy of a
/// canonical notebook: a `.ipynb` file passing through a
/// `translations/<locale>/` segment with at least one path segment after the
/// locale. The bare `translations/<locale>` dir, non-`.ipynb` localization
/// files, and a `translations`-prefixed sibling dir do NOT match.
pub fn is_translated_notebook_copy(rel_path: &str) -> bool {
    let norm = rel_path.replace('\\', "/");
    if !norm.ends_with(".ipynb") {
        return false;
    }
    let segments: Vec<&str> = norm.split('/').collect();
    // Find a `translations` segment that is followed by a locale segment and
    // then at least one more segment (the notebook path under that locale).
    segments
        .iter()
        .position(|s| *s == "translations")
        .is_some_and(|idx| idx + 2 < segments.len())
}

#[cfg(test)]
#[path = "jupyter_dedup_tests.rs"]
mod tests;
