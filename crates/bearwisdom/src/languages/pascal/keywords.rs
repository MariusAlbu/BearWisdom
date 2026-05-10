// =============================================================================
// pascal/keywords.rs — Pascal/Delphi syntactic keywords
//
// This list contains only true grammar tokens — reserved words and
// pseudo-identifiers that appear as syntactic elements in the Pascal
// grammar, not in any library declaration. Library types, procedures,
// and classes (RTL, VCL, LCL, SysUtils, etc.) are discovered by the
// freepascal_runtime ecosystem walker and resolved via the symbol index.
// =============================================================================

/// True Pascal/Delphi syntactic keywords — reserved words, directives,
/// and pseudo-identifiers (Result, Self, inherited, nil, True, False)
/// that the grammar treats as tokens rather than ordinary identifiers.
pub(crate) const KEYWORDS: &[&str] = &[
    // Program structure
    "program", "unit", "library", "package",
    "uses", "interface", "implementation", "initialization", "finalization",
    // Declarations
    "var", "const", "type", "label", "threadvar", "resourcestring",
    "procedure", "function", "constructor", "destructor", "operator",
    "class", "object", "record", "interface",
    "property", "published", "public", "protected", "private", "strict",
    "abstract", "virtual", "override", "overload", "reintroduce",
    "dynamic", "message", "static", "inline", "assembler",
    "external", "forward", "stdcall", "cdecl", "pascal", "register",
    "safecall", "winapi",
    // Type modifiers
    "array", "of", "set", "file", "string",
    "packed", "dispinterface",
    "generic", "specialize",
    // Control flow
    "begin", "end",
    "if", "then", "else",
    "case", "of",
    "while", "do",
    "repeat", "until",
    "for", "to", "downto",
    "with",
    "goto",
    // Exception handling
    "try", "except", "finally", "raise", "on",
    // Boolean / logic operators (appear as keywords in the grammar)
    "and", "or", "not", "xor", "in", "is", "as",
    "div", "mod", "shl", "shr",
    // Pseudo-identifiers that the grammar treats as keywords
    "nil", "True", "False",
    "Self", "inherited", "Result",
    // Compiler-directive-adjacent keywords
    "out", "default", "name", "index", "read", "write",
    "stored", "nodefault",
];
