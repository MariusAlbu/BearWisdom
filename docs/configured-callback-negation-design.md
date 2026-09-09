# Configured callback negation

1. [profile data] Capture boolean-negation syntax from a grammar-owned node/operator/operand form.
2. [generic] Add a source-owned negation recipe that retains every operand read and selector through portable/cold hydration.
3. [generic] Evaluate the complete operand before supplying a boolean result; private/missing/invalid members stay incomplete evidence.
4. [generic] Fold nested negation parity while deriving supported typeof and truthiness predicates from parameter BindingIds.
5. [generic] Require two-sided narrowing for inferred predicates; primitive domains containing both truthy and falsy values must not be treated as disjoint.
6. [generic] Keep explicit return annotations authoritative and all contextual callback writes private until overload selection.
7. [generic] Preserve nominal/literal/type IDs in truthiness partitions, with bounded recursion and unknown/generic barriers.
8. [generic] Verify independent TypeScript 5.9.3 callback types, predicate slots and selected-overload origins, including negative operand controls.
9. [generic] Verify poisoned source metadata, portable/cold snapshots, provider edits/deletion and the complete constructor-to-filter-to-has cascade.
10. [generic] Capture exact real constructor/overload origins with compiler binding order before closing broader difference<T> roadmap work, then remeasure the immutable cohort.

The callback recipe now retains negation operands and their BindingIds. Evaluation
validates the operand before returning boolean; inferred predicates fold negation
parity and retain the parameter's source slot and exact narrowed TypeId. Explicit
return annotations suppress inferred predicates. Broad primitive truthiness tests
remain ordinary boolean callbacks; unsupported generic partitions remain unknown.
The binding epoch is 69 and extractor schema is 81.

`verify_callback_negation.mjs` passes 22 pinned TypeScript 5.9.3 cases in script and
module scopes: 40 exact callback returns, 18 inferred predicates, 40 selected
overload source declarations, 40 exact result types and two diagnostic negatives.
The engine matrix checks those labels after portable-cache hydration into a shifted
arena, poisoned display names/signatures/call operands, and cold database reload.
Provider selection, incompatible method edits, deletion, recovery and foreign
context rejection pass. The failing-before method-negation fixture is retained.

The independent `capture_constructor_cascade.mjs` adapter replays the frozen
configuration and verifies every read against its original hash. Its supplemental
artifact records all 187 compiler source-binding positions and the exact three
selected signatures in `difference<T>`: the iterable Set constructor in file 9 at
6641, the ordinary Array.filter overload in file 59 at 67211, and Set.has in file 5
at 4089. The arrow returns boolean without a predicate. This adds evidence without
changing the frozen manifest. Exact constructor selection in the engine remains
separate work; agreement between candidates' result types does not identify the
compiler-selected declaration.

Verification: 2,145 selected library tests passed, followed by the additional
provider edit/deletion test (2,146 total); all seven targeted integrations passed.
The native Windows file-budget/identity audit covered 344 production files and
136 resolver/type files with zero violations. `git diff --check` passed.

Frozen-cohort reports are
`resolution-documents/2026-09-09-query-core-callback-negation-program-report.json`
(575 correct, 40 declaration-kind disagreements, 75 unresolved) and
`resolution-documents/2026-09-09-query-core-callback-negation-legacy-report.json`
(557 correct, 40 disagreements, 93 unresolved). Both match every field of the
iterable-inference reports except elapsed time, with zero fresh/cold snapshot
changes. The configured recall remains 83.33%; the 99% gate remains open.

The retained real-source probe confirms the next dependency: the configured
initializer at bytes 446..474 is `Set<T>`, but `FileLookup` reports no local type
for `excludeSet` at the subsequent filter call. The callback recipe is correctly
`Not(Call(Read(506..516), selector 517, Read(521..522)))`; filter applicability
therefore remains unknown. Source-owned initializer publication to the local
BindingId must preserve annotation precedence, declaration order, reassignment
boundaries, and snapshot invalidation. Do not force a boolean callback while its
captured receiver remains untyped. Exact constructor dispatch is also still open.
