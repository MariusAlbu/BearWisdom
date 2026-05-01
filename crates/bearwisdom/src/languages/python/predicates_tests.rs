use super::predicates;

#[test]
fn django_methods_not_classified_as_python_builtin() {
    // Django model convenience methods used to be in `is_python_builtin`,
    // causing false fast-exits across non-Django Python codebases (where
    // `save`, `delete`, `clean` are common method names). Django belongs
    // in its own ecosystem walker, indexed when `pyproject.toml` declares
    // it.
    for name in &[
        "refresh_from_db",
        "save",
        "delete",
        "get_absolute_url",
        "full_clean",
        "clean",
    ] {
        assert!(
            !predicates::is_python_builtin(name),
            "{name:?} should not be classified as a python builtin",
        );
    }
}

#[test]
fn real_python_builtins_still_classified() {
    // Sanity: actual Python builtins / stdlib methods still match.
    for name in &[
        "len", "print", "isinstance", "range", "ValueError",
        // stdlib str instance methods
        "split", "strip", "lower",
        // unittest assert helpers (stdlib)
        "assertEqual", "assertTrue",
    ] {
        assert!(
            predicates::is_python_builtin(name),
            "{name:?} must remain a python builtin",
        );
    }
}
