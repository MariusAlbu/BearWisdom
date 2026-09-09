1. [evidence] The new rustc-validated negative occurrence calls `absent()` beside an unrelated module declaring it; the recorded engine snapshot incorrectly targets that module's function.
2. [root cause] Source capture records an unconfigured external-root recipe for the missing bare value. `namespace_input::use_fact` discards non-local Unconfigured results, so `SemanticModel` receives no binding fact.
3. [cascade] The old same-file name ladder can then select an unrelated declaration and propagate its return type into downstream member calls or local initializer types.
4. [generic] Treat a syntactically attested bare call's namespace lookup as authoritative even when its provider is unknown. Preserve the existing numeric recipe and unknown result; do not fabricate a declaration ID.
5. [generic] Reuse `Use.local`'s existing no-fallback behavior, documenting its authoritative-source meaning. Lexical local/parameter facts still take precedence over namespace facts in `FileLookup`.
6. [profile data] Existing call and identifier CST forms decide which source occurrences are covered. No language-name branch, spelling comparison, new runtime compiler dependency or builtin list is added.
7. [boundaries] Qualified external paths retain their current behavior; complete implicit prelude supply and unconfigured external binding are still open. Unknown bare values must not use unrestricted name search as substitute evidence.
8. [persistence] Increment binding/extraction epochs because the authoritative flag is persisted through portable source metadata and cold snapshots; existing numeric recipe lowering remains unchanged.
9. [tests] First pin the failing compiler-labelled negative and captured authoritative fact; verify rejected factory-root cascades, valid local/imported calls, source filtering and fresh/cold persistence.
10. [gates] Retain the original observed false-positive snapshot, require its negative site to improve, and run all oracle/namespace/engine and focused incremental integration checks without claiming the entire F1 or 99% gate complete.

Verified failing first: `compiler_rejected_bare_call_cannot_select_an_unrelated_module_function`
resolved the compiler-rejected call to the unrelated function. It now passes fresh
and cold. The factory cascade test covers direct/nested calls and local initializers,
with explicit imports as positive controls, including explicit generic syntax.
Binding epoch 18 / extraction schema 38 invalidate old source-binding payloads.
The selected 2,083-test core/profile run passes; implicit prelude supply and complete
unconfigured external semantics remain open, not justified by bare-name fallback.
