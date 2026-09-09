# Configured method inheritance

1. [generic] Preserve declaration and signature origins while comparing source-owned inherited methods.
2. [generic] Reconstruct complete Callable evidence from Bound signatures, retaining generics, defaults, constraints, receiver/optional/rest flags and unknown barriers.
3. [generic] Keep equality rules for conflicting multiple bases; use directional signature assignment for a child's explicit method override.
4. [generic] Add a method comparison mode to the existing callable relation, including method parameter bivariance and callback variance rules; optional CallablePolicy.bivariant_methods is attested by compiler ingestion and absent on old inputs.
5. [generic] Prove covariant return types through selected nominal inheritance applications with GenericParamId substitution and effective alias evaluation.
6. [generic] Traverse declared base applications with bounded, cycle-checked state; do not infer compatibility from names or unrelated same-head arguments.
7. [generic] Keep candidate views private and rely on monotone settlement to revoke every dependent proof when an invalid ancestor or source disappears.
8. [generic] Capture independent TypeScript 5.9.3 diagnostics and exact member-call origins for recursive, generic, optional, overloaded and invalid override cases.
9. [generic] Verify poisoned portable/cold inputs, missing policy, selected providers, edits/deletion, foreign contexts and recursive inheritance rejection.
10. [generic] Trace ArrayIterator and the full real Set initializer after targeted regressions; keep remaining structural/iterable inference and constructor-order gaps explicit.

## Evidence and remaining work

Implemented at binding epoch 67 / extractor schema 80. The new optional CallablePolicy.bivariant_methods field is attested at compiler-options ingestion; serde-defaulted old inputs retain the existing equality proof. Method declarations use the selected policy for parameter variance and nested callback comparison. The [TypeScript 2.6 release notes](https://www.typescriptlang.org/docs/handbook/release-notes/typescript-2-6.html) describe the method exemption from strict function parameter variance.

Bound signatures retain source origins, generic parameter IDs, constraints, defaults, receiver/optional/rest flags and unknown barriers. Positive return compatibility can follow selected declared base applications, with substituted arguments and bounded cycle checks. This evidence stays private until monotone merge settlement revokes invalid ancestors and their dependents. Same-head argument similarity alone supplies no variance proof; unrelated nominal heads remain unknown.

The independent TypeScript 5.9.3 verifier checks 31 fixtures in both script and module scopes: 18 legal cases, 13 diagnostic-negative cases, no legal abstentions, and six exact source call declaration/signature labels. Cases include recursive/alias/generic ancestors, unique-symbol selectors, invalid referenced ancestors, narrowed/widened/disjoint returns and parameters, optional members/parameters, generic alpha equivalence and constraints, callbacks, overload coverage, conflicting multiple bases, cycles and fixed tuple rests.

Four new engine regressions cover the complete self-to-read-to-touch cascade with poisoned display metadata and shifted portable arenas; fresh/cold admission for every compiler fixture; configuration-only method-policy changes; selected-provider retargeting, source edits, stale source hashes, deletion and foreign contexts. A three-hop chain can produce multiple extracted references ending at one compiler selector. Every such reference must match its exact compiler target before deduplicating the coverage map. The selected library suite passed 2,142 tests, with 32 ignored.

The frozen real-source provenance probe now admits ArrayIterator row 14170. Its computed iterator method returns ArrayIterator<T>; its inherited next method preserves the rest parameter and IteratorResult<T, undefined> result, including the separate optional return/throw members. Both IteratorObject providers remain selected, the global BuiltinIteratorReturn intrinsic remains undefined, and the Node namespace namesake retains its separate conditional alias body.

```ts
interface Base<T> { self(): Base<T>; read(): T }
interface Catalog<T> extends Base<T> { self(): Catalog<T> } // [generic] passes method/declared-ancestor proof
catalog.self().read().touch(); // [generic] passes all three exact source target checks
new Set(array2); // [generic] still unknown in program_constructor_arguments::infer for Array<T> -> Iterable<T>
```

The readonly-array constructor candidate remains applicable as Set<caller T>, but the iterable candidate and complete initializer remain unknown. The next root requires source-member structural inference, full callable/optional-member assignment, and the iterator's union-of-tuples rest evidence. An ancestry projection may contribute inference candidates; final applicability must independently prove the complete relation. Constructor selection also requires real compiler binding order, which the retained manifest currently lacks; physical row IDs are not order evidence. Callback negation and the complete difference<T> cascade remain open.

All seven targeted integrations passed: TypeScript, JavaScript and Rust resolution corpora, plus per-file manifest and per-package context tests. The native Windows budget/identity audit passed across 340 changed production files and 132 scoped resolver/type files, with zero budget violations or new string-identity patterns. Git diff whitespace checks passed.

The public CallablePolicy field is serde-defaulted. Neither AlphaT's source tree nor Lynx's impact source tree directly constructs Program or CallablePolicy. Required offline/locked sibling compile attempts used workspace-local target directories: AlphaT fails dependency resolution at yanked der 0.8.0 through ureq/ort; Lynx requires a lockfile update. Neither reaches source compilation; sibling compatibility remains unverified and their files are unchanged.

## Frozen cohort results

The immutable manifest remains `resolution-documents/2026-09-07-query-core-configured-compiler-manifest.json`, SHA-256 `44158824a3f91fdeda2d5e5da8a422054e178c76de29857c2f855ffe36a5110c`: 187 supplied sources, 24 selected files, 843 compiler calls, 690 labelled targets, zero compiler diagnostics.

Configured report: `resolution-documents/2026-09-09-query-core-method-heritage-program-report.json`, SHA-256 `ae3e0a466c8eda72f4c3941cf2c1a779460955966d1a9effe8d07f4f6731a19f`, elapsed 49,510 ms. Fresh/cold: 575 correct, 40 kind-only incorrect, 75 unresolved, 83.3333% recall, 93.4959% strict precision, zero snapshot changes, gate ineligible. Deep equality excluding only elapsed_ms confirms every retained occurrence and report field is unchanged from the compiler-intrinsics report.

Legacy report: `resolution-documents/2026-09-09-query-core-method-heritage-legacy-report.json`, SHA-256 `da7766f5b18f63d2a3b1efe72096e5106bf9540f029dc20ead08ac3c8530633d`, elapsed 52,474 ms. Fresh/cold: 557 correct, 40 kind-only incorrect, 93 unresolved, zero snapshot changes, gate ineligible. Deep equality excluding only elapsed_ms confirms every field and retained occurrence is unchanged from the compiler-intrinsics legacy report. The manifest hash was reverified unchanged.

The roadmap preserves all existing lines and appends one completed method-heritage child. Its broader iterator/constructor/callback parent remains open. Current raw checkbox count is 124/216 (57.4%); this is task count, not an effort estimate or readiness score.
