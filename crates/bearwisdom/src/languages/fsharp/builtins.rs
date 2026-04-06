// =============================================================================
// fsharp/builtins.rs — F# builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(sym_kind, "method" | "function" | "constructor" | "test" | "class"),
        EdgeKind::Inherits => matches!(sym_kind, "class"),
        EdgeKind::Implements => matches!(sym_kind, "class" | "interface"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "interface" | "enum" | "type_alias" | "function" | "variable"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class" | "function"),
        _ => true,
    }
}

/// F# built-in functions and standard library types always in scope.
pub(super) fn is_fsharp_builtin(name: &str) -> bool {
    matches!(
        name,
        // Core I/O and formatting
        "printfn"
            | "printf"
            | "sprintf"
            | "eprintfn"
            // Error / control
            | "failwith"
            | "failwithf"
            | "invalidArg"
            | "invalidOp"
            | "raise"
            | "reraise"
            // Utility functions
            | "ignore"
            | "id"
            | "fst"
            | "snd"
            | "not"
            | "defaultArg"
            // Option / Result
            | "Option"
            | "Some"
            | "None"
            | "Result"
            | "Ok"
            | "Error"
            | "Choice"
            // Collection modules
            | "List"
            | "Array"
            | "Seq"
            | "Map"
            | "Set"
            | "String"
            // Primitive types
            | "int"
            | "float"
            | "decimal"
            | "string"
            | "bool"
            | "char"
            | "byte"
            | "sbyte"
            | "int16"
            | "uint16"
            | "int32"
            | "uint32"
            | "int64"
            | "uint64"
            | "nativeint"
            | "unativeint"
            | "single"
            | "double"
            | "unit"
            | "obj"
            | "exn"
            // Async / Task
            | "async"
            | "task"
            | "Async"
            | "Task"
            | "Observable"
            | "Event"
            | "MailboxProcessor"
            | "Agent"
            | "Lazy"
            // Operators / reflection
            | "ref"
            | "box"
            | "unbox"
            | "typeof"
            | "typedefof"
            | "sizeof"
            | "nameof"
            | "lock"
            | "using"
            | "dispose"
    )
}
