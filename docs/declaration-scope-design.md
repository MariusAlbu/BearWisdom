# Declaration-scope identity — F1 continuation
1. Core modules: lexical ingestion, scoped cache, semantic model and chain roots.
2. [profile data] Describe named function/class declarations in the TS/JS syntax table.
3. [generic] Register those names in their declaring scope, separately from parameter/body scopes.
4. [generic] Correlate full declaration source anchors to exact extracted rows, preserving numeric IDs.
5. [generic] Prebind bare constructors and chain-root occurrences, not only bare local calls.
6. [generic] Carry declaration kind with source-bound IDs; do not classify declarations by name.
7. [generic] Root local classes on Type::Decl and local functions on their ID-keyed return metadata.
8. [generic] Capture root type arguments once; canonicalize explicit function returns to GenericParamId templates at ingestion.
9. [generic] Preserve local shadow barriers and uncertainty when declaration/type capture is missing.
10. Verify compiler-labelled shadowing, sibling classes, hoisting and callable-return cascades before broader regressions.

```ts
function f(callback: () => Alpha) {
  { function callback(): Beta { return new Beta(); }
    callback(); // ❌ baseline [generic] nested function declaration is absent from lexical binding graph
  }
}
// ⚠️ [generic] Named expressions, imports, declaration merging, type-annotation binding
// and non-call identifier extraction remain separate requirements, not implied successes.
```

Integration follow-up: wrapper-return inference must carry the exact owner row ID,
with source-bound callees read by ID and candidate agreement by TypeId. Preserve
the old name-bucket compatibility output for unmigrated consumers; do not copy
a name-bucket winner back into unrelated declaration IDs. Named nested declarations
must not have their calls emitted again under the outer declaration's ownership.
