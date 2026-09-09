1. [generic] Move CallArg and SourceSpan into a focused module, preserving their public re-export paths and legacy serialized variants.
2. [generic] Add an identifier-use variant carrying only its exact source span, never a spelling for semantic lookup.
3. [profile data] TS/JS shared argument syntax extraction emits spans recursively; parentheses preserve the underlying expression address.
4. [generic] Visit argument identifiers on refs and every chain segment at ingestion, binding SourceSpan -> BindingId at the use's own lexical scope.
5. [generic] Keep argument-use IDs separate from call-root expression IDs and member-selector IDs to prevent domain collisions.
6. [generic] Read binding facts and callable provenance at the argument position without moving or borrowing the resolver's ambient cursor.
7. [generic] Materialize class values as Constructor(Decl) and function values as canonical Function parameter/return TypeIds by exact declaration ID.
8. [generic] Missing source/row/type evidence abstains; legacy variants remain for unmigrated languages, with no new name fallback for source-addressed arguments.
9. [generic] Pin constructor/function-value cascades, sibling/block/callback argument identity, missing-span non-interference, recursive extraction and serialization.
10. [generic] Verify compiler-labelled calls, resolver and incremental tests, public consumers and source/ID gates; do not claim full import/CFG or 99% completeness.

The member-function argument fixture exposed a second cause: wrapper inference
guarded only the extractor's optional return slot, then overwrote a source-bound
method return with Unknown. The guard now also respects the exact declaration's
source return recipe. This protects declared contracts independently of legacy
signature-parser completeness.

Initializer prepasses must also use the file-local ID environment. The direct
field pass now owns member-call initializers even when the extractor emits no
separate TypeRef marker. Ingestion records the exact declaration slot plus the
initializer expression and called-selector bytes; nested calls in arrays or
closures are not evidence of the whole field's value. The chain pass uses that
same call occurrence (including its arguments), not an argument-less TypeRef
copy. Unmigrated files retain their existing legacy path. Unknown is tested as
a Type variant, not by formatting a type back into a string.

The pass-isolation test exposed another ownership bridge: extracted variable
initializer calls can belong to the enclosing function. A single numeric
occurrence index joins the captured initializer address back to its declaration
slot, independently of that legacy source attribution. The field pass creates
one file environment per candidate-bearing file. The sequential chain prepass
still rebuilds it per candidate; amortizing that environment without losing
prior-declaration updates is a performance follow-up, not a measured speed win.
