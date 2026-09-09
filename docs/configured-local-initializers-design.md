# Configured initializer results for local bindings

1. [profile ingestion] Capture the exact simple-variable declaration token for each source initializer; properties and destructuring retain separate ownership.
2. [generic] Map the captured token span to its lexical BindingId, retaining the full initializer SignatureId as the configured query address.
3. [generic] Expose authoritative configured initializer results by source/signature identity, including missing and invalid results.
4. [generic] Install inferred local declaration types only from the selected source snapshot; preserve explicit annotations and initialization availability.
5. [generic] Keep unsupported initializer evidence from restoring an extractor constructor-name seed, and retain immutable inferred contracts across assignments.
6. [generic] Carry constructor results through captured closure reads, candidate-local callback inference, selected overload results and downstream member targets.
7. [generic] Verify same-spelled sibling/block/closure bindings, pre-initializer reads, incompatible operands, provider changes, stale source hashes and cold hydration.
8. [generic] Retain compiler-selected constructor/filter/has origins as independent labels; identical candidate return types do not establish dispatch.
9. [generic] Verify the frozen real difference<T> cascade and all retained positive/negative occurrence labels before remeasuring configured and legacy cohorts.
10. [generic] Extend exact constructor dispatch with source owner/order evidence after the local publication path is proved; preserve unknown competitors and configuration barriers.

The initializer now records its exact local declaration token. That token remains
separate from the whole declaration's SignatureId and its optional navigation row.
Configured views attest the signature/token pair, and the lexical cache installs
the resulting TypeId through the token's BindingId. Missing constructor evidence
becomes an authoritative unknown; unrelated initializer families retain their
existing evaluators. Explicit annotations and availability checks remain ahead
of inferred initializer types. Binding epoch 70 and extractor schema 82 invalidate
older evidence.

Nine TypeScript 5.9.3 compiler fixtures cover script and module scopes, with 24
exact local-type labels, four exact constructor signatures and 14 exact downstream
call signatures. The engine tests replay them with poisoned names/signatures/call
operands, portable-cache hydration into a shifted arena, and cold database reload.
They include sibling/block/closure identity, annotation precedence, pre-initializer
reads, preserved non-union contracts after reassignment, invalid arguments and
abstract constructors. Wrong signature/token pairs are rejected. Provider selection,
edits, deletion, recovery, consumer edits and stale source hashes also pass.

The initial cascade fixture failed before implementation because its local
constructor BindingId had no type. It now resolves both concrete and caller-generic
constructor-to-callback-to-filter-to-member chains. The retained real-source probe
also yields the same Set<T> TypeId for the initializer and local excludeSet binding,
then selects Array.filter in lib.es5.d.ts at byte 67211 with the caller's Array<T>
return identity. Exact constructor dispatch remains open: the initializer evaluator
still uses agreement among applicable result types, which does not prove which
constructor declaration TypeScript selected.

The configured frozen cohort improves from 575 to 581 correct calls out of 690
(84.20% recall), with 40 unchanged declaration-kind disagreements and 69 unresolved.
Every changed fresh/cold occurrence is unresolved-to-correct, and there are zero
snapshot differences. The six corrected sites are `originalQueue.forEach` at
notifyManager.ts byte 1094, and queriesObserver.ts bytes 517 (`excludeSet.has`),
7442/7799 (`prevObserversMap.get`), 7564 (`prevObserversMap.set`), and 7832 (`shift`).
Thus the retained originalQueue alias also benefits from publishing a proved
source-owned read initializer. The configured report is
`resolution-documents/2026-09-09-query-core-local-initializers-program-report.json`.
The immutable compiler manifest and supplemental selected-signature labels remain
unchanged; this is still a development cohort and does not satisfy the 99% gate.

The legacy report (`resolution-documents/2026-09-09-query-core-local-initializers-legacy-report.json`)
matches the previous complete report except elapsed time: 557 correct, 40 kind
disagreements, 93 unresolved and zero fresh/cold differences. The retained real
probe now asserts the exact compiler filter and has declarations for every
extracted emission, counting unique source selectors, and passes. Verification
totals 2,148 selected library tests plus the manual real-source probe and seven
integrations; the final combined local/provider/real-probe run passes all three.
The native Windows audit covers 345 production files and 137 resolver/type files
with zero budget or identity violations; `git diff --check` passes.
