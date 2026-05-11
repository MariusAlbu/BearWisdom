// =============================================================================
// fsharp/predicates.rs — F# builtin and helper predicates
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

/// Fallback external namespace check when no ProjectContext is available.
/// Matches common .NET namespace roots (System, Microsoft, etc.).
pub(super) fn is_external_namespace_fallback(ns: &str) -> bool {
    let root = ns.split('.').next().unwrap_or(ns);
    matches!(
        root,
        "System" | "Microsoft" | "Newtonsoft" | "Serilog" | "NLog"
            | "AutoMapper" | "FluentValidation" | "MediatR" | "Polly"
            | "NSubstitute" | "Moq" | "FakeItEasy" | "Xunit" | "NUnit"
            | "Giraffe" | "Saturn" | "Suave" | "Fable" | "Elmish"
            | "FSharp" | "FsToolkit" | "Thoth" | "Fantomas"
            | "Expecto" | "Fake" | "BenchmarkDotNet" | "Fornax"
            | "Argu" | "FParsec" | "FsCheck"
    )
}
