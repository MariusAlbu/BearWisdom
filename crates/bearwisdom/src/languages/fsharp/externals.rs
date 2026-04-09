// =============================================================================
// fsharp/externals.rs — F# external symbol list
// =============================================================================
//
// Bare names and qualified module-function names that are always external to
// any F# project.  These include FSharp.Core primitives, collection module
// functions, Expecto test framework, and common DU constructors.

use std::collections::HashSet;

/// F# core and standard library names that are always external.
pub(crate) const EXTERNALS: &[&str] = &[
    // -----------------------------------------------------------------------
    // Core I/O and formatting
    // -----------------------------------------------------------------------
    "printfn", "printf", "sprintf", "eprintfn",
    // -----------------------------------------------------------------------
    // Error / control
    // -----------------------------------------------------------------------
    "failwith", "failwithf", "invalidArg", "invalidOp", "nullArg",
    "raise", "reraise",
    // -----------------------------------------------------------------------
    // Utility
    // -----------------------------------------------------------------------
    "ignore", "id", "fst", "snd", "not", "defaultArg", "defaultof",
    // -----------------------------------------------------------------------
    // Option / Result constructors and module names
    // -----------------------------------------------------------------------
    "Some", "None", "Ok", "Error", "ValueSome", "ValueNone",
    "Option", "Result", "Choice",
    // -----------------------------------------------------------------------
    // Collection module names (bare — used as type annotations or opened)
    // -----------------------------------------------------------------------
    "List", "Array", "Seq", "seq", "list", "array",
    "Map", "Set", "String", "Dictionary", "ResizeArray",
    // -----------------------------------------------------------------------
    // Async / Task
    // -----------------------------------------------------------------------
    "Async", "Task", "ValueTask", "async", "task",
    "Observable", "Event", "MailboxProcessor", "Agent", "Lazy",
    // -----------------------------------------------------------------------
    // Reflection / type introspection
    // -----------------------------------------------------------------------
    "ref", "box", "unbox", "typeof", "typedefof", "sizeof", "nameof",
    "lock", "using", "dispose",
    // -----------------------------------------------------------------------
    // Primitive types (used as conversion functions and type annotations)
    // -----------------------------------------------------------------------
    "int", "float", "decimal", "string", "bool", "char",
    "byte", "sbyte", "int8", "int16", "uint16", "int32", "uint32",
    "int64", "uint64", "nativeint", "unativeint", "single", "double",
    "float32", "bigint", "unit", "obj", "exn", "void",
    // -----------------------------------------------------------------------
    // Seq module — qualified function calls
    // -----------------------------------------------------------------------
    "Seq.map", "Seq.filter", "Seq.fold", "Seq.foldBack",
    "Seq.iter", "Seq.iteri", "Seq.collect",
    "Seq.head", "Seq.tail", "Seq.last", "Seq.length",
    "Seq.empty", "Seq.isEmpty", "Seq.toList", "Seq.toArray",
    "Seq.ofList", "Seq.ofArray", "Seq.append", "Seq.concat",
    "Seq.choose", "Seq.tryFind", "Seq.find", "Seq.exists", "Seq.forall",
    "Seq.take", "Seq.skip", "Seq.zip", "Seq.mapi", "Seq.countBy",
    "Seq.groupBy", "Seq.sortBy", "Seq.distinct", "Seq.truncate",
    "Seq.singleton", "Seq.init", "Seq.initInfinite", "Seq.unfold",
    "Seq.pairwise", "Seq.windowed", "Seq.reduce",
    "Seq.sum", "Seq.sumBy", "Seq.max", "Seq.maxBy",
    "Seq.min", "Seq.minBy", "Seq.average", "Seq.averageBy",
    // -----------------------------------------------------------------------
    // List module — qualified function calls
    // -----------------------------------------------------------------------
    "List.map", "List.filter", "List.fold", "List.foldBack",
    "List.iter", "List.iteri", "List.collect",
    "List.head", "List.tail", "List.last", "List.length",
    "List.empty", "List.isEmpty", "List.rev",
    "List.append", "List.concat", "List.choose",
    "List.tryFind", "List.find", "List.exists", "List.forall",
    "List.take", "List.skip", "List.zip", "List.mapi",
    "List.countBy", "List.groupBy", "List.sortBy", "List.distinct",
    "List.truncate", "List.singleton", "List.init", "List.unfold",
    "List.pairwise", "List.windowed", "List.reduce",
    "List.sum", "List.sumBy", "List.max", "List.maxBy",
    "List.min", "List.minBy", "List.average", "List.averageBy",
    "List.partition", "List.splitAt", "List.item", "List.tryItem",
    "List.indexed", "List.allPairs", "List.exactlyOne", "List.tryExactlyOne",
    // -----------------------------------------------------------------------
    // Array module — qualified function calls
    // -----------------------------------------------------------------------
    "Array.map", "Array.filter", "Array.fold", "Array.foldBack",
    "Array.iter", "Array.iteri", "Array.collect",
    "Array.length", "Array.empty", "Array.isEmpty", "Array.rev",
    "Array.append", "Array.concat", "Array.choose",
    "Array.tryFind", "Array.find", "Array.exists", "Array.forall",
    "Array.take", "Array.skip", "Array.zip", "Array.mapi",
    "Array.sortBy", "Array.create", "Array.init", "Array.zeroCreate",
    "Array.copy", "Array.sub", "Array.blit", "Array.fill",
    "Array.toList", "Array.ofList", "Array.toSeq", "Array.ofSeq",
    "Array.reduce", "Array.sum", "Array.sumBy",
    "Array.max", "Array.maxBy", "Array.min", "Array.minBy",
    "Array.average", "Array.averageBy", "Array.partition", "Array.splitAt",
    "Array.item", "Array.tryItem", "Array.indexed", "Array.singleton",
    // -----------------------------------------------------------------------
    // Map module — qualified function calls
    // -----------------------------------------------------------------------
    "Map.empty", "Map.add", "Map.remove", "Map.find", "Map.tryFind",
    "Map.containsKey", "Map.ofList", "Map.toList", "Map.ofSeq", "Map.toSeq",
    "Map.ofArray", "Map.toArray", "Map.map", "Map.filter",
    "Map.fold", "Map.foldBack", "Map.iter", "Map.partition",
    "Map.exists", "Map.forall", "Map.count", "Map.isEmpty",
    "Map.keys", "Map.values", "Map.change", "Map.tryGetValue",
    // -----------------------------------------------------------------------
    // Set module — qualified function calls
    // -----------------------------------------------------------------------
    "Set.empty", "Set.add", "Set.remove", "Set.contains",
    "Set.ofList", "Set.toList", "Set.ofSeq", "Set.toSeq",
    "Set.ofArray", "Set.toArray", "Set.map", "Set.filter",
    "Set.fold", "Set.foldBack", "Set.iter", "Set.partition",
    "Set.exists", "Set.forall", "Set.count", "Set.isEmpty",
    "Set.union", "Set.intersect", "Set.difference",
    "Set.isSubset", "Set.isSuperset", "Set.singleton",
    // -----------------------------------------------------------------------
    // String module (FSharp.Core)
    // -----------------------------------------------------------------------
    "String.concat", "String.IsNullOrEmpty", "String.IsNullOrWhiteSpace",
    "String.length", "String.init", "String.collect", "String.map",
    "String.filter", "String.exists", "String.forall", "String.replicate",
];

/// Dependency-gated framework globals for F#.
pub(crate) fn framework_globals(deps: &HashSet<String>) -> Vec<&'static str> {
    let mut globals = Vec::new();

    // Expecto test framework
    if deps.contains("Expecto") || deps.contains("expecto") {
        globals.extend(EXPECTO_GLOBALS);
    }

    // FsUnit assertion library
    if deps.contains("FsUnit") || deps.contains("FsUnit.Xunit") || deps.contains("FsUnit.MsTest") {
        globals.extend(FSUNIT_GLOBALS);
    }

    globals
}

static EXPECTO_GLOBALS: &[&str] = &[
    "testCase",
    "testCaseAsync",
    "testList",
    "testListAsync",
    "testSequenced",
    "testSequencedGroup",
    "testAsync",
    "testProperty",
    "testPropertyWithConfig",
    "pending",
    "pendingTest",
    "ftestCase",
    "ftestList",
    "ptestCase",
    "ptestList",
    "Expect.equal",
    "Expect.notEqual",
    "Expect.isTrue",
    "Expect.isFalse",
    "Expect.isNull",
    "Expect.isNotNull",
    "Expect.isSome",
    "Expect.isNone",
    "Expect.isEmpty",
    "Expect.isNotEmpty",
    "Expect.throws",
    "Expect.throwsT",
    "Expect.isOk",
    "Expect.isError",
    "Expect.contains",
    "Expect.sequenceEqual",
    "Expect.stringContains",
    "Expect.stringStarts",
    "Expect.stringEnds",
    "Expect.hasCountOf",
    "Expect.wantOk",
    "Expect.wantError",
    "runTests",
    "runTestsWithArgs",
    "runTestsWithCLIArgs",
    "runTestsInAssembly",
    "defaultConfig",
];

static FSUNIT_GLOBALS: &[&str] = &[
    "should", "equal", "not'", "contain", "haveLength", "be",
    "ofType", "greaterThan", "lessThan", "greaterThanOrEqualTo",
    "lessThanOrEqualTo",
];

