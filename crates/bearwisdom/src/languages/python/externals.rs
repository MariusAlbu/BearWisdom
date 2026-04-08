use std::collections::HashSet;

/// Runtime globals always external for Python.
pub(crate) const EXTERNALS: &[&str] = &[
    // Synthetic type annotations
    "__type__", "__metadata__",
];

/// Dependency-gated framework globals for Python.
pub(crate) fn framework_globals(deps: &HashSet<String>) -> Vec<&'static str> {
    let mut globals = Vec::new();

    if deps.contains("pytest") {
        globals.extend(PYTEST_GLOBALS);
    }

    globals
}

const PYTEST_GLOBALS: &[&str] = &[
    "fixture", "mark", "parametrize", "raises", "approx", "monkeypatch",
    "capsys", "capfd", "caplog", "tmp_path", "tmp_path_factory",
    "request", "pytestconfig", "cache", "doctest_namespace",
    "recwarn", "capfdbinary", "capsysbinary",
];
