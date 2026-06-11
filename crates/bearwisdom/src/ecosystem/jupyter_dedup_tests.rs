use super::*;

#[test]
fn translated_notebook_copy_is_detected() {
    // `translations/<lang>/<canonical-rel-path>/X.ipynb` is a per-locale copy
    // of the canonical notebook at `<canonical-rel-path>/X.ipynb`.
    assert!(is_translated_notebook_copy(
        "translations/ar/2-Regression/1-Tools/notebook.ipynb"
    ));
    assert!(is_translated_notebook_copy(
        "translations/zh-cn/4-Classification/solution/R/lesson_10-R.ipynb"
    ));
    // Nested under a section's own translations dir.
    assert!(is_translated_notebook_copy(
        "8-Reinforcement/1-QLearning/translations/de/notebook.ipynb"
    ));
}

#[test]
fn canonical_notebook_is_not_a_copy() {
    // The canonical notebook lives outside any translations/ subtree.
    assert!(!is_translated_notebook_copy(
        "2-Regression/1-Tools/notebook.ipynb"
    ));
    assert!(!is_translated_notebook_copy(
        "4-Classification/1-Introduction/solution/R/lesson_10-R.ipynb"
    ));
}

#[test]
fn translations_localization_files_are_not_notebook_copies() {
    // Non-notebook localization payload directly under translations/ — not a
    // duplicated notebook, must not be reclassified.
    assert!(!is_translated_notebook_copy("translations/en.json"));
    assert!(!is_translated_notebook_copy("translations/index.js"));
    assert!(!is_translated_notebook_copy(
        "translations/ar/README.ar.md"
    ));
}

#[test]
fn translations_must_be_followed_by_locale_then_notebook() {
    // A notebook directly inside translations/ with no locale segment between
    // it and the file is not the per-locale copy pattern.
    assert!(!is_translated_notebook_copy(
        "translations/notebook.ipynb"
    ));
}

#[test]
fn non_notebook_under_translations_locale_is_ignored() {
    // Only .ipynb duplicates are deduped; other translated files stay as-is.
    assert!(!is_translated_notebook_copy(
        "translations/ar/2-Regression/README.md"
    ));
}

#[test]
fn prefix_sibling_dir_named_like_translations_is_not_matched() {
    // `translations-archive` is a sibling, not the translations subtree.
    assert!(!is_translated_notebook_copy(
        "translations-archive/ar/notebook.ipynb"
    ));
}

#[test]
fn windows_backslash_paths_normalize() {
    assert!(is_translated_notebook_copy(
        "translations\\ar\\2-Regression\\notebook.ipynb"
    ));
}
