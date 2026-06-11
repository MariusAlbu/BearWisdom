// Tests for the F# keyword set after the third-party-API purge. The dotted
// FSharp.Core module functions (`List.map`, …) and the Expecto test API were
// removed: FSharp.Core resolves via the dotnet-stdlib walker, Expecto via the
// NuGet walker. Only language-spec primitives, core types, prelude intrinsics,
// computation-expression keywords, and generic-param placeholders remain.

use super::keywords::KEYWORDS;

#[test]
fn language_spec_names_kept() {
    // Primitives + core FSharp.Core prelude types/cases + intrinsic functions.
    for name in [
        "int", "string", "bool", "unit", "obj", "decimal", "Ok", "Error", "Some", "None",
        "Result", "Option", "Async", "Task", "List", "Array", "Seq", "Map", "Set", "printfn",
        "failwith", "ignore", "id", "fst", "snd", "box", "unbox", "nameof",
    ] {
        assert!(
            KEYWORDS.contains(&name),
            "language-spec name `{name}` was dropped"
        );
    }
}

#[test]
fn dotted_module_api_purged() {
    // Module-qualified FSharp.Core functions resolve through the dotnet-stdlib
    // walker, not a hand-maintained list.
    for name in [
        "List.map",
        "List.filter",
        "Seq.fold",
        "Array.zeroCreate",
        "Option.bind",
        "Result.map",
        "Async.Sleep",
        "Task.WhenAll",
        "String.concat",
        "Map.ofList",
        "Set.union",
    ] {
        assert!(
            !KEYWORDS.contains(&name),
            "dotted FSharp.Core API `{name}` survived the purge"
        );
    }
}

#[test]
fn expecto_api_purged() {
    // Expecto is a NuGet package — its API must not live in the keyword set.
    for name in [
        "Expect.equal",
        "Expect.isTrue",
        "Expect.throws",
        "testList",
        "testCase",
        "testCaseAsync",
        "testProperty",
        "ftestCase",
        "ptestCase",
    ] {
        assert!(
            !KEYWORDS.contains(&name),
            "Expecto API `{name}` survived the purge"
        );
    }
}

#[test]
fn no_dotted_names_remain() {
    // A blanket structural guard: the keyword set is a closed bare-name
    // language surface — no module-qualified (dotted) entries belong here.
    for name in KEYWORDS {
        assert!(
            !name.contains('.'),
            "dotted entry `{name}` must not be in the F# keyword set"
        );
    }
}
