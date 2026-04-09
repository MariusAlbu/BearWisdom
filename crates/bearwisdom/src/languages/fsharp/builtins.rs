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
    // Qualified module-function calls: Seq.map, List.filter, Array.fold, etc.
    if is_fsharp_qualified_builtin(name) {
        return true;
    }
    matches!(
        name,
        // -----------------------------------------------------------------------
        // Core I/O and formatting
        // -----------------------------------------------------------------------
        "printfn"
            | "printf"
            | "sprintf"
            | "eprintfn"
            // -----------------------------------------------------------------------
            // Error / control
            // -----------------------------------------------------------------------
            | "failwith"
            | "failwithf"
            | "invalidArg"
            | "invalidOp"
            | "nullArg"
            | "raise"
            | "reraise"
            // -----------------------------------------------------------------------
            // Utility functions
            // -----------------------------------------------------------------------
            | "ignore"
            | "id"
            | "fst"
            | "snd"
            | "not"
            | "defaultArg"
            | "defaultof"
            // -----------------------------------------------------------------------
            // Option / Result discriminated unions
            // -----------------------------------------------------------------------
            | "Option"
            | "Some"
            | "None"
            | "Result"
            | "Ok"
            | "Error"
            | "Choice"
            | "ValueSome"
            | "ValueNone"
            // -----------------------------------------------------------------------
            // Collection modules (unqualified — after `open` or as type names)
            // -----------------------------------------------------------------------
            | "List"
            | "Array"
            | "Seq"
            | "seq"
            | "list"
            | "array"
            | "Map"
            | "Set"
            | "String"
            | "Dictionary"
            | "ResizeArray"
            // -----------------------------------------------------------------------
            // Primitive types
            // -----------------------------------------------------------------------
            | "int"
            | "float"
            | "decimal"
            | "string"
            | "bool"
            | "char"
            | "byte"
            | "sbyte"
            | "int8"
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
            | "float32"
            | "bigint"
            | "unit"
            | "obj"
            | "exn"
            | "void"
            // -----------------------------------------------------------------------
            // Async / Task / concurrent types
            // -----------------------------------------------------------------------
            | "async"
            | "task"
            | "Async"
            | "Task"
            | "ValueTask"
            | "Observable"
            | "Event"
            | "MailboxProcessor"
            | "Agent"
            | "Lazy"
            // -----------------------------------------------------------------------
            // Reflection / type introspection
            // -----------------------------------------------------------------------
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
    )
}

/// Qualified F# stdlib calls (e.g. `Seq.map`, `List.filter`, `Map.empty`).
fn is_fsharp_qualified_builtin(name: &str) -> bool {
    matches!(
        name,
        // -----------------------------------------------------------------------
        // Seq module
        // -----------------------------------------------------------------------
        "Seq.map"
            | "Seq.filter"
            | "Seq.fold"
            | "Seq.foldBack"
            | "Seq.iter"
            | "Seq.iteri"
            | "Seq.collect"
            | "Seq.head"
            | "Seq.tail"
            | "Seq.last"
            | "Seq.length"
            | "Seq.empty"
            | "Seq.isEmpty"
            | "Seq.toList"
            | "Seq.toArray"
            | "Seq.ofList"
            | "Seq.ofArray"
            | "Seq.append"
            | "Seq.concat"
            | "Seq.choose"
            | "Seq.tryFind"
            | "Seq.find"
            | "Seq.exists"
            | "Seq.forall"
            | "Seq.take"
            | "Seq.skip"
            | "Seq.zip"
            | "Seq.mapi"
            | "Seq.countBy"
            | "Seq.groupBy"
            | "Seq.sortBy"
            | "Seq.distinct"
            | "Seq.truncate"
            | "Seq.singleton"
            | "Seq.init"
            | "Seq.initInfinite"
            | "Seq.unfold"
            | "Seq.pairwise"
            | "Seq.windowed"
            | "Seq.reduce"
            | "Seq.sum"
            | "Seq.sumBy"
            | "Seq.max"
            | "Seq.maxBy"
            | "Seq.min"
            | "Seq.minBy"
            | "Seq.average"
            | "Seq.averageBy"
            // -----------------------------------------------------------------------
            // List module
            // -----------------------------------------------------------------------
            | "List.map"
            | "List.filter"
            | "List.fold"
            | "List.foldBack"
            | "List.iter"
            | "List.iteri"
            | "List.collect"
            | "List.head"
            | "List.tail"
            | "List.last"
            | "List.length"
            | "List.empty"
            | "List.isEmpty"
            | "List.rev"
            | "List.append"
            | "List.concat"
            | "List.choose"
            | "List.tryFind"
            | "List.find"
            | "List.exists"
            | "List.forall"
            | "List.take"
            | "List.skip"
            | "List.zip"
            | "List.mapi"
            | "List.countBy"
            | "List.groupBy"
            | "List.sortBy"
            | "List.distinct"
            | "List.truncate"
            | "List.singleton"
            | "List.init"
            | "List.unfold"
            | "List.pairwise"
            | "List.windowed"
            | "List.reduce"
            | "List.sum"
            | "List.sumBy"
            | "List.max"
            | "List.maxBy"
            | "List.min"
            | "List.minBy"
            | "List.average"
            | "List.averageBy"
            | "List.partition"
            | "List.splitAt"
            | "List.item"
            | "List.tryItem"
            | "List.indexed"
            | "List.allPairs"
            | "List.exactlyOne"
            | "List.tryExactlyOne"
            // -----------------------------------------------------------------------
            // Array module
            // -----------------------------------------------------------------------
            | "Array.map"
            | "Array.filter"
            | "Array.fold"
            | "Array.foldBack"
            | "Array.iter"
            | "Array.iteri"
            | "Array.collect"
            | "Array.length"
            | "Array.empty"
            | "Array.isEmpty"
            | "Array.rev"
            | "Array.append"
            | "Array.concat"
            | "Array.choose"
            | "Array.tryFind"
            | "Array.find"
            | "Array.exists"
            | "Array.forall"
            | "Array.take"
            | "Array.skip"
            | "Array.zip"
            | "Array.mapi"
            | "Array.sortBy"
            | "Array.create"
            | "Array.init"
            | "Array.zeroCreate"
            | "Array.copy"
            | "Array.sub"
            | "Array.blit"
            | "Array.fill"
            | "Array.toList"
            | "Array.ofList"
            | "Array.toSeq"
            | "Array.ofSeq"
            | "Array.reduce"
            | "Array.sum"
            | "Array.sumBy"
            | "Array.max"
            | "Array.maxBy"
            | "Array.min"
            | "Array.minBy"
            | "Array.average"
            | "Array.averageBy"
            | "Array.partition"
            | "Array.splitAt"
            | "Array.item"
            | "Array.tryItem"
            | "Array.indexed"
            | "Array.singleton"
            // -----------------------------------------------------------------------
            // Map module
            // -----------------------------------------------------------------------
            | "Map.empty"
            | "Map.add"
            | "Map.remove"
            | "Map.find"
            | "Map.tryFind"
            | "Map.containsKey"
            | "Map.ofList"
            | "Map.toList"
            | "Map.ofSeq"
            | "Map.toSeq"
            | "Map.ofArray"
            | "Map.toArray"
            | "Map.map"
            | "Map.filter"
            | "Map.fold"
            | "Map.foldBack"
            | "Map.iter"
            | "Map.partition"
            | "Map.exists"
            | "Map.forall"
            | "Map.count"
            | "Map.isEmpty"
            | "Map.keys"
            | "Map.values"
            | "Map.change"
            | "Map.tryGetValue"
            // -----------------------------------------------------------------------
            // Set module
            // -----------------------------------------------------------------------
            | "Set.empty"
            | "Set.add"
            | "Set.remove"
            | "Set.contains"
            | "Set.ofList"
            | "Set.toList"
            | "Set.ofSeq"
            | "Set.toSeq"
            | "Set.ofArray"
            | "Set.toArray"
            | "Set.map"
            | "Set.filter"
            | "Set.fold"
            | "Set.foldBack"
            | "Set.iter"
            | "Set.partition"
            | "Set.exists"
            | "Set.forall"
            | "Set.count"
            | "Set.isEmpty"
            | "Set.union"
            | "Set.intersect"
            | "Set.difference"
            | "Set.isSubset"
            | "Set.isSuperset"
            | "Set.singleton"
            // -----------------------------------------------------------------------
            // String module (FSharp.Core)
            // -----------------------------------------------------------------------
            | "String.concat"
            | "String.IsNullOrEmpty"
            | "String.IsNullOrWhiteSpace"
            | "String.length"
            | "String.init"
            | "String.collect"
            | "String.map"
            | "String.filter"
            | "String.exists"
            | "String.forall"
            | "String.replicate"
    )
}
