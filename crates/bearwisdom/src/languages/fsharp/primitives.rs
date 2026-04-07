// =============================================================================
// fsharp/primitives.rs — F# primitive types
// =============================================================================

/// Primitive and built-in type names for F#.
pub(crate) const PRIMITIVES: &[&str] = &[
    // Primitives
    "int", "int8", "int16", "int32", "int64",
    "uint8", "uint16", "uint32", "uint64",
    "float", "float32", "double", "decimal",
    "bool", "char", "string", "unit", "obj", "byte",
    "sbyte", "nativeint", "unativeint", "bigint",
    "exn", "void",
    // Core types
    "Ok", "Error", "Some", "None", "ValueSome", "ValueNone",
    "Result", "Option", "ValueOption", "Choice",
    "Async", "Task", "ValueTask", "Lazy",
    "Map", "Set", "Dictionary", "List", "Array", "Seq",
    "ResizeArray", "HashSet", "Queue", "Stack",
    "IEnumerable", "IEnumerator", "IDisposable", "IComparable",
    "Event", "DelegateEvent", "MailboxProcessor",
    // Functions
    "printfn", "printf", "sprintf", "failwith", "failwithf",
    "invalidArg", "invalidOp", "nullArg", "raise", "reraise",
    "ignore", "id", "fst", "snd", "not", "defaultArg",
    "hash", "compare", "min", "max", "abs", "sign", "pown", "sqrt",
    "infinity", "nan", "typeof", "typedefof", "sizeof", "nameof",
    "unbox", "box", "ref", "incr", "decr", "exit",
    "stdin", "stdout", "stderr",
    "async", "task", "lock", "using", "seq", "query", "yield", "return",
    // Computation expression keywords
    "__app__", "Invoke", "AsTask",
    // List module
    "List.map", "List.filter", "List.fold", "List.iter", "List.head",
    "List.tail", "List.length", "List.rev", "List.sort", "List.sortBy",
    "List.collect", "List.choose", "List.exists", "List.forall",
    "List.find", "List.tryFind", "List.groupBy", "List.zip", "List.unzip",
    "List.mapi", "List.iteri", "List.contains", "List.distinct",
    "List.isEmpty", "List.empty", "List.singleton", "List.append",
    "List.concat", "List.sum", "List.sumBy", "List.average", "List.averageBy",
    "List.max", "List.maxBy", "List.min", "List.minBy",
    // Array module
    "Array.map", "Array.filter", "Array.fold", "Array.iter",
    "Array.length", "Array.sort", "Array.create", "Array.init", "Array.zeroCreate",
    // Seq module
    "Seq.map", "Seq.filter", "Seq.fold", "Seq.iter",
    "Seq.head", "Seq.tail", "Seq.length", "Seq.empty",
    // Option module
    "Option.map", "Option.bind", "Option.defaultValue",
    "Option.isSome", "Option.isNone", "Option.get", "Option.iter",
    // Result module
    "Result.map", "Result.bind", "Result.mapError", "Result.isOk", "Result.isError",
    "Result.Ok",
    // Async module
    "Async.RunSynchronously", "Async.Start", "Async.StartAsTask",
    "Async.AwaitTask", "Async.Sleep", "Async.Parallel", "Async.Catch",
    // Task module
    "Task.FromResult", "Task.WhenAll", "Task.Delay",
    // String module
    "String.concat", "String.IsNullOrEmpty", "String.IsNullOrWhiteSpace",
    // Map/Set modules
    "Map.ofList", "Map.ofSeq", "Map.find", "Map.tryFind", "Map.add",
    "Map.remove", "Map.containsKey", "Map.empty", "Map.toList",
    "Set.ofList", "Set.add", "Set.remove", "Set.contains", "Set.empty",
    "Set.union", "Set.intersect", "Set.difference",
    // Expecto test framework
    "Expect.equal", "Expect.isTrue", "Expect.isFalse",
    "Expect.isEmpty", "Expect.isNonEmpty", "Expect.hasLength",
    "Expect.contains", "Expect.containsAll", "Expect.throws", "Expect.throwsT",
    "testList", "testCase", "testCaseAsync", "testProperty", "testSequenced",
    "ftestCase", "ptestCase",
    // Generic type params
    "T", "U", "K", "V",
];
