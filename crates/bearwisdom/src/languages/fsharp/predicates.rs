// =============================================================================
// fsharp/predicates.rs — F# builtin and helper predicates
// =============================================================================

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
