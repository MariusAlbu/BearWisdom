// =============================================================================
// php/keywords.rs — PHP primitive types and language constructs
// =============================================================================

/// Primitive and built-in type names for PHP.
pub(crate) const KEYWORDS: &[&str] = &[
    "int", "float", "string", "bool", "array", "object", "null", "void", "mixed", "never",
    "callable", "iterable", "self", "static", "parent", "true", "false",
];

/// PHP language constructs — reserved keywords invoked with call-like syntax
/// but which are NOT functions (the engine sees them as call targets). Closed
/// set fixed by the language grammar; never a library/runtime API list.
pub(crate) const CONSTRUCTS: &[&str] = &[
    "isset", "empty", "unset", "echo", "print", "list", "eval", "exit", "die", "include",
    "include_once", "require", "require_once", "array", "__halt_compiler",
];
