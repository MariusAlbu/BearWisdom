1. [generic] Reuse the source namespace pass's ScopeId/NameId graph for lexical locals; do not build another spelling-based local resolver.
2. [profile data] Supply local declaration, parameter, pattern, scope and initializer shapes from Rust syntax data.
3. [generic] Allocate distinct ordered BindingIds for shadowing declarations; an initializer sees the previous binding until the new declaration becomes visible.
4. [generic] Correlate declarations, writes and full initializer calls by exact source spans; retain the existing shared typed-fact cache.
5. [generic] Bind local annotations to declaration/module IDs through source-addressed recipes; known missing imports remain unknown, never name-based type evidence.
6. [generic] Exclude initializer-owned synthetic TypeRefs from declared-type inference and carry failed inference as an explicit unknown by binding identity.
7. [generic] Use source-bound bare/import call facts where available; local value bindings take precedence over namespace facts.
8. [generic] Preserve snapshot-local ID allocation across filtering/reload and invalidate old binding metadata when allocation changes.
9. [generic] Test the whole rejected-constructor/local/member cascade, positive cascades, same-block shadowing, sibling scopes, annotations and cold reload.
10. [generic] Verify broader regression cohorts and report unsupported pattern/CFG/language semantics explicitly; this work does not establish the representative 99% gate.

## Implemented slice — 2026-09-07

```rust
let p = AliasDoc::new(); // ✅ [generic] namespace recipe → constructor ID → return TypeId
p.touch();              // ✅ [generic] occurrence → local BindingId → exact member ID
// With a missing export and an indexed AliasDoc namesake, both calls remain unresolved.

let p = a::Doc::new();   // ✅ [generic] first ordered binding
let p = p.convert();    // ✅ [generic] initializer reads the first binding, writes a distinct second ID
p.touch();              // ✅ [generic] the second binding carries convert's return TypeId

fn f(p: &Alias) {
    p.touch();          // ✅ [generic] supported annotation head uses the namespace declaration ID
}
```

`namespace_locals` consumes the namespace ingestion pass's scope/name arena into
the shared lexical cache. It captures ordered declarations, parameters, simple
annotation heads, identifier arguments and exact initializer/write ownership.
Local bindings cannot become global candidates; closures can capture them, while
function/module item boundaries stop local capture. Qualified type paths remain
separate from value namesakes. Language syntax is data in the Rust namespace forms.

`namespace_occurrences` stamps supported call-chain selectors directly from CST
nodes **before** binding capture. Previously Rust segments still had zero offsets
at that point; later canonicalization filled them with text search. Matching
initializer writes now uses one per-file numeric address index. Same-root calls
are ordered by selector address as well as root address, without spelling keys.

Source-owned local types exclude synthetic initializer TypeRefs from the legacy
declared-type pass. Failed RHS inference records an explicit Unknown fact by
BindingId; profile-selected static declared types still take precedence. Thus a
missing `Vec::new()` result cannot erase an explicit `Vec<Widget>` annotation.
Simple annotation recipes survive module-graph persistence and fresh
metadata recapture after contract filtering. Binding epoch **6** rejects old
module recipes; extraction payload schema remains 32 because filtered metadata
is rebuilt from source, without synthesizing body rows.

## Evidence and limits

The failing-first multi-file test initially produced an unresolved constructor
but a `touch` edge to the decoy declaration (row 8 in that test run). The repaired
test asserts exact constructor/member targets in the positive case and no such
edges in the negative cases, both fresh and cold. Additional tests cover same-block
shadowing, sibling functions/blocks, closures, bare/imported factories, annotations,
type/value collisions, repeated selectors, failure facts and metadata rebuilding.

Three older flow tests used placeholder column-zero declaration rows. Their
fixtures now use real identifier coordinates; expected binding slots and captured
annotation values are unchanged. One cache test now explicitly requires Unknown
after a failed RHS instead of retaining the prior inferred type.

The selected core cohort passes **1,914 tests, 16 ignored**. The TypeScript 5.9.3
oracle verification still checks **213 labels** (79 module + 134 scope). These are
not new independent Rust labels or a representative 99% result.

Final integration verification passes **12 tests** across incremental indexing,
per-file/per-package manifests and the general, JavaScript and Rust resolution
corpora. Rust retains **32/32 legacy patterns plus the exact constructor/member
ID assertions**; the three separately reported known-red probes remain open.
The source-file budget and resolver string-identity audit checks 110 changed
production files with zero violations; `git diff --check` passes.

An extra source-coordinate correlation regression verifies that an extractor's
existing closure-parameter Variable row is adopted, not duplicated as a new
Parameter row leaving the original outside the lexical visibility fence.

Still open: complete type-use reference navigation, generic/qualified composite
annotation recipes, struct/tuple-constructor and value-alias initializer forms,
pattern projection and match/loop/conditional scopes, contextual closures and
CFG joins, configured external/stdlib semantics, trait dispatch, and semantic
dependent-edge invalidation. Unsupported forms and unconfigured external paths
still have legacy boundaries. The prelude's per-candidate environment rebuilding
also remains a separate performance task. This slice does not claim complete
Rust lexical/type semantics or an engine-wide elimination of string bridges.
