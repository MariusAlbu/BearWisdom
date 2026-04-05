// =============================================================================
// indexer/primitives.rs — Language primitive type sets
//
// Returns the set of built-in/primitive type names for each supported language.
// These are never in the project's symbol index and should be classified as
// external (or simply filtered) rather than emitting unresolved edges.
// =============================================================================

/// Return the slice of primitive/built-in type names for the given language
/// identifier. Returns an empty slice for unrecognised languages.
pub fn primitives_for_language(lang: &str) -> &'static [&'static str] {
    match lang {
        "typescript" | "javascript" | "tsx" | "jsx" | "svelte" | "astro" | "vue" | "angular" => &[
            "string", "number", "boolean", "void", "null", "undefined", "any", "never",
            "object", "symbol", "bigint", "unknown", "String", "Number", "Boolean", "Object",
            "Array", "Function", "Symbol", "RegExp", "Date", "Error", "Promise", "Map", "Set",
        ],
        "java" | "kotlin" | "scala" | "groovy" => &[
            "int", "long", "float", "double", "boolean", "char", "byte", "short", "void",
            "Integer", "Long", "Float", "Double", "Boolean", "Character", "Byte", "Short",
            "String", "Object", "Void",
        ],
        "csharp" | "fsharp" | "vbnet" => &[
            "int", "long", "float", "double", "bool", "char", "byte", "string", "object",
            "void", "decimal", "dynamic", "short", "ushort", "uint", "ulong", "sbyte",
            "nint", "nuint",
        ],
        "rust" => &[
            "i8", "i16", "i32", "i64", "i128", "u8", "u16", "u32", "u64", "u128",
            "f32", "f64", "bool", "char", "str", "usize", "isize", "String", "Vec",
            "Option", "Result", "Box", "Rc", "Arc", "Self",
        ],
        "python" => &[
            "int", "float", "str", "bool", "None", "bytes", "list", "dict", "tuple",
            "set", "type", "object", "complex", "frozenset", "memoryview", "range",
            "True", "False",
        ],
        "go" => &[
            "int", "int8", "int16", "int32", "int64", "uint", "uint8", "uint16", "uint32",
            "uint64", "float32", "float64", "bool", "string", "byte", "rune", "error", "any",
            "complex64", "complex128", "uintptr",
        ],
        "swift" => &[
            "Int", "Int8", "Int16", "Int32", "Int64", "UInt", "UInt8", "UInt16", "UInt32",
            "UInt64", "Float", "Double", "Bool", "String", "Character", "Void", "Any",
            "AnyObject", "Self", "Optional", "Array", "Dictionary", "Set",
        ],
        "dart" => &[
            "int", "double", "num", "bool", "String", "List", "Map", "Set", "dynamic",
            "void", "Null", "Object", "Future", "Stream", "Iterable", "Type", "Function",
        ],
        "php" => &[
            "int", "float", "string", "bool", "array", "object", "null", "void", "mixed",
            "never", "callable", "iterable", "self", "static", "parent", "true", "false",
        ],
        "ruby" => &[
            "Integer", "Float", "String", "Symbol", "Array", "Hash", "NilClass", "TrueClass",
            "FalseClass", "Numeric", "Object", "BasicObject", "Kernel", "Comparable",
            "Enumerable",
        ],
        "elixir" => &[
            "integer", "float", "atom", "string", "boolean", "list", "tuple", "map",
            "nil", "pid", "reference", "binary", "function", "port",
        ],
        _ => &[],
    }
}

/// Build a `HashSet<String>` from the primitives for a given language.
/// Convenience wrapper for callers that need owned-string set membership.
pub fn primitives_set_for_language(lang: &str) -> std::collections::HashSet<String> {
    primitives_for_language(lang)
        .iter()
        .map(|s| s.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typescript_primitives_include_string_and_number() {
        let p = primitives_for_language("typescript");
        assert!(p.contains(&"string"));
        assert!(p.contains(&"number"));
        assert!(p.contains(&"Promise"));
    }

    #[test]
    fn rust_primitives_include_i32_and_str() {
        let p = primitives_for_language("rust");
        assert!(p.contains(&"i32"));
        assert!(p.contains(&"str"));
        assert!(p.contains(&"Option"));
        assert!(p.contains(&"Result"));
    }

    #[test]
    fn go_primitives_include_error() {
        let p = primitives_for_language("go");
        assert!(p.contains(&"error"));
        assert!(p.contains(&"string"));
    }

    #[test]
    fn unknown_language_returns_empty() {
        assert!(primitives_for_language("brainfuck").is_empty());
    }

    #[test]
    fn tsx_same_as_typescript() {
        assert_eq!(
            primitives_for_language("tsx"),
            primitives_for_language("typescript")
        );
    }
}
