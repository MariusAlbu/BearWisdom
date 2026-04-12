/// Runtime globals always external for Python.
pub(crate) const EXTERNALS: &[&str] = &[
    // Synthetic type annotations
    "__type__", "__metadata__",
];
