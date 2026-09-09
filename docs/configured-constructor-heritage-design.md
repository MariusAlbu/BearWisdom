1. Preserve additive inherited construct overloads; compiler evidence disproves treating them as named-method overrides.
2. Compare source-owned constructor facets structurally with the ordinary callable variance policy, distinct from method bivariance.
3. Preserve parameter arity, optional/rest flags, source generic binders, constraints, defaults and covariant result evidence.
4. Keep each inherited and own signature's real source instance, span and optional navigation declaration.
5. Verify how constructor overload groups combine across own, single-base, multiple-base and diamond inheritance.
6. Leave private inheritance settlement responsible for rejecting incompatible, unknown and cyclic bases and dependents.
7. Make no grammar or per-language spelling changes; refactor generic proof boundaries only where needed.
8. Reproduce the retained constructor_variance_heritage_retained_gap before implementing the change.
9. Pin TypeScript diagnostics, exact constructor choices, result types and downstream declarations with negative/state controls.
10. Run targeted library/integration checks and compare every frozen fresh/cold benchmark observation before closing the task.

Compiler evidence: TypeScript 5.9.3 typescript.js:62355 concatenates own and inherited construct signatures. Its signature relation at line 69033 exempts Constructor declarations and methods from strict variance, but not ConstructSignature declarations. The retained failure is therefore an inheritance-model error, and full constructor-type variance remains a separate obligation. Store constructor facets separately from keyof members; keep call facets and unsupported inference positions as completeness barriers.

```ts
interface Item { touch(): number }
interface Other { other(): string }
interface Base { new(value: number): Item }
interface Factory extends Base { new(value: string): Other }
declare const Build: Factory;
new Build(1).touch();   // ✅ [generic] private heritage keeps Base's overload; source constructor dispatch reaches Item.touch.
new Build('x').other(); // ✅ [generic] the own overload retains its distinct result and source signature.

interface Wide { new(value: number | string): Item }
interface Narrow { new(value: number): Item }
declare const wide: Wide, narrow: Narrow;
const accepted: Narrow = wide; // ✅ [generic] constructor_groups uses ordinary callable contravariance.
const rejected: Wide = narrow; // ✅ [generic] strict parameter policy rejects; method bivariance cannot admit it.

interface GenericWide { new<T extends number | string>(value: T): Item }
interface GenericNarrow { new<U extends number>(value: U): Item }
declare const genericWide: GenericWide;
const genericNarrow: GenericNarrow = genericWide; // ⚠️ [generic] legal compiler case; differing generic constraint domains still need contextual instantiation.

interface Many { new(...values: number[]): Item }
declare const many: Many;
const one: Narrow = many; // ⚠️ [generic] legal compiler case; callable_parameters currently proves finite tuple rests only.
```

The implementation stores full source-owned constructor facets in nominal surfaces, independently of named/keyof members. Private heritage retains every own and inherited constructor overload, validates complete operands, and preserves incompatible named-property, unknown and cyclic-base rejection. Structural argument proofs compare every required constructor against the source overload group using the existing ordinary callable relation. Alpha-owned generic binders, constraints/defaults, optional parameters, finite tuple rests, nested callbacks and results keep their existing proof rules. Constructor inventory validation requires callable configuration only for a constrained default proof; the older inherited `Result<string>` initializer remains valid without variance configuration.

Binding epoch is 74; extractor schema remains 83 because serialized extraction facts did not change. No grammar hooks or spelling-based semantic recovery were added. Generic inference from constructor facets, hidden call facets and unsupported signature forms remain explicit barriers. The broad full-variance roadmap item stays open, with this completed inheritance and finite-signature slice recorded underneath it.

Validation on 2026-09-09: 2,161 selected library tests passed, with 32 existing ignored probes. Four new tests cover compiler cases, source-context rejection, policy changes and provider/consumer edits/deletion. The final 43-case matrix passed after two retained-gap controls were appended. TypeScript 5.9.3 independently verified both script and module scopes: 172 inventory signatures, 30 selected constructor origins, 30 exact constructor results, 46 assignments and 30 downstream calls, with 12 diagnostic-negative cases. The previous 23-case constructor selection verifier and 54-case initializer verifier also passed; the latter checked 39 exact types and signature presences. Tests include portable shifted arenas, poisoned display metadata, absent constructor navigation rows, and persisted cold rebuilds. The native budget/ID audit passed for all 353 changed production files.

Frozen Query Core remeasurement used the unchanged 187-file/24-selected-file manifest with 843 compiler calls and 690 labelled calls. The configured result remains 585 correct, 40 declaration-kind disagreements and 65 unresolved; legacy remains 557 correct, 40 kind disagreements and 93 unresolved. Every fresh/cold observation and every report field except elapsed time matches the preceding conditional-receiver reports. Both snapshots have zero changes. Immutable evidence:

- `resolution-documents/2026-09-09-query-core-constructor-heritage-program-report.json`: SHA256 `f573d20af7f655a3373e142a2149ef876b449f93b7ffe9c85be4497fdca0d9c7`; 48,641 ms.
- `resolution-documents/2026-09-09-query-core-constructor-heritage-legacy-report.json`: SHA256 `d199dfce75e24107a6a79c416134b78aecd37d872b514cbb101e676905796fa0`; 53,231 ms.
- `resolution-documents/2026-09-09-query-core-constructor-heritage-comparison.json` records complete before/after hashes and counts for both modes.

All seven integrations passed across the TypeScript, JavaScript and Rust resolution corpora, per-file manifests and per-package context. The existing build warnings remain outside this change. No workspace-wide build, lint or formatting pass was run.

Next: start with the compiler-valid `unbounded_rest_variance_retained_gap` and `generic_constraint_domain_retained_gap` fixtures. Extend generic callable argument proofs with compiler-owned negative and configuration controls; do not erase rest arity or generic constraint domains to make these tests pass.

## Unbounded array-rest variance closeout

TypeScript 5.9.3 compares ordinary array rests as repeated positional element types. A source `(...values: number[]) => R` can therefore satisfy `(value: number) => R`, accept an empty target, or compare against longer fixed targets; strict parameter variance still rejects a narrower repeated element, while the relaxed ordinary-callable policy can admit it. Finite tuple and union-of-tuple rests retain their correlated remainder proof instead of being widened into this rule.

The evaluator now obtains an unbounded rest's element only through the selected program's canonical mutable-array declaration role. Missing, stale, foreign-context and module-local namesake declarations remain unknown, with no display-name fallback. Portable fixture providers live in a separate `arrays.d.ts` source so script and module consumers exercise the same global identity boundary. Mixed finite-correlated and unbounded tails remain conservative until a compiler-backed cross-form proof is added.

Binding epoch is 75; extractor schema remains 83. The compiler relation matrix now has 61 cases: 37 positives, 21 diagnostic-backed negatives, three invalid-source barriers and three legal abstentions. The 43-case constructor-heritage matrix still verifies both scopes, 172 inventory signatures, 30 selected origins/results/downstream calls, 46 assignments and 12 diagnostic negatives; `unbounded_rest_variance_retained_gap` is now supported. The existing constructor-selection and initializer oracles also pass their 23 and 54 cases. The selected Rust cohort passes 2,161 tests with 32 existing ignored probes, and all seven integrations pass.

Frozen Query Core remains unchanged at every occurrence. The configured report has 585 correct, 40 declaration-kind disagreements and 65 unresolved labels; the legacy report has 557 correct, 40 kind disagreements and 93 unresolved. Both have exact fresh/cold parity and zero snapshot changes, and each complete report equals its constructor-heritage predecessor after removing `elapsed_ms`. Immutable evidence:

- `resolution-documents/2026-09-09-query-core-unbounded-rest-program-report.json`: SHA256 `67dd7cb1a1d78cea16a056f67bbfe6d8ac0b542d8261df2d3acfbc8177d20e0d`; 53,045 ms.
- `resolution-documents/2026-09-09-query-core-unbounded-rest-legacy-report.json`: SHA256 `6411931362fcb26aeeec37a2471e70d3d366a86b5f1ebc7c84ed29e3e6804f66`; 33,790 ms.

Next: close `generic_constraint_domain_retained_gap` through contextual generic instantiation while preserving source-owned constraint identities, invalid-domain rejection and configuration/edit/deletion/cold barriers.

## Contextual generic-signature instantiation closeout

TypeScript 5.9.3 does not compare generic call signatures by declaration-order alpha renaming. `compareSignaturesRelated` first instantiates a generic source signature in the target signature's context: parameter positions contribute inference candidates, the return contributes a lower-priority candidate, and the inferred source arguments must satisfy their source constraints. This permits a source `<T extends number | string>(value: T) => T` to cover a target `<U extends number>(value: U) => U`, rejects the inverse and disjoint domains, and allows binder reordering or multiple source binders to map to one target binder.

The callable evaluator now performs that contextual source instantiation with source-owned `GenericParamId` values. Target binders stay rigid and retain their selected constraint domains; unconstrained target binders cannot satisfy a proper source constraint. Inference follows fixed parameters, configured mutable-array applications, nested tuples and supported wrappers, then the predicate assertion or return. Repeated compatible candidates merge toward the wider proved domain. Independent conflicting candidates, contextual finite-rest correlation and unsupported shapes remain unknown. The instantiated source is then checked by the existing arity, variance, predicate and result relation, so inference does not bypass ordinary callable policy.

Binding epoch is 76; extractor schema remains 83 because no serialized extraction facts changed. The TypeScript 5.9.3 callable oracle now has 76 cases: 46 positives, 27 diagnostic-backed negatives, three invalid-source barriers and two legal abstentions. It covers wider/narrower, unconstrained and disjoint domains, reordered and dependent binders, parameter-, return-, tuple- and configured-array inference, monomorphic candidates, and repeated-candidate controls. The 43-case constructor-heritage oracle still verifies both scopes, 172 inventory signatures, 30 selected origins/results/downstream calls, 46 assignments and 12 diagnostic negatives; `generic_constraint_domain_retained_gap` is now supported. Portable shifted arenas, poisoned display metadata, fresh/cold restoration, callable-policy changes and the existing constructor provider/edit/deletion state gates all pass.

The selected Rust cohort passes 2,161 tests with 32 existing ignored probes, and all seven integrations pass. The file-budget and ID-discipline audits pass. Frozen Query Core is unchanged at every occurrence and has exact fresh/cold parity with zero snapshot changes. Both complete reports equal their unbounded-rest predecessors after removing only `elapsed_ms`:

- `resolution-documents/2026-09-09-query-core-generic-context-program-report.json`: SHA256 `1d40be114239fe06b530934720ca9a136e781d0ffe77ed6af8a770f0f2e5f714`; 40,713 ms; 585 correct, 40 declaration-kind disagreements and 65 unresolved of 690.
- `resolution-documents/2026-09-09-query-core-generic-context-legacy-report.json`: SHA256 `04f197ffdb4a5c90980f4bda86919b59e654da826206f8f621ce07e649bca483`; 37,155 ms; 557 correct, 40 declaration-kind disagreements and 93 unresolved of 690.

The full construct-signature variance parent remains open for contextual finite/mixed rest tails, more complex repeated-candidate inference, call facets and inference from constructor facets.

## Contextual generic array-rest inference closeout

TypeScript 5.9.3 infers a direct generic source rest from the complete target remainder. For a source `<T extends number[]>(...values: T)`, a fixed target contributes a tuple such as `[number]`, while an unbounded target contributes its original configured array type. Fixed source parameters before the generic rest are inferred positionally first. The inferred rest candidate remains a complete tuple or array so later result inference can reuse the same source-owned `GenericParamId`.

The callable evaluator now preserves both the element type and original configured array rest, builds compiler-shaped contextual tails, and instantiates direct generic source rests before ordinary signature variance runs. Tuple candidates are checked item by item against the configured array constraint. When a candidate violates that constraint, the evaluator follows the compiler's fallback to the declared constraint and lets the normal arity, strictness, parameter and result checks decide the relation. This covers empty, fixed, optional, tuple-rest and array targets, fixed prefixes, aliases, readonly configured arrays, rigid constrained target binders and a tuple-valued return without weakening constructor or callable identity rules.

Rest evidence is validated before broad target shortcuts. Unconstrained or scalar generic rests remain invalid, missing configured providers remain unknown, and unsupported tuple-union or dependent constraints do not gain a result through `any` or `unknown`. Mixed fixed-plus-array target tails remain unknown because the type model does not yet encode variadic tuples. Tuple/dependent source constraints and repeated rest-versus-return candidates that require compiler inference priority also remain explicit abstentions.

Binding epoch is 77; extractor schema remains 83 because no serialized extraction shape changed. The TypeScript 5.9.3 callable oracle has 95 cases: 60 positives, 31 diagnostic-backed negatives, four invalid-source barriers and six legal abstentions. The cross-kind rest oracle has 72 cases over ordinary call, construct and method signatures in script and module scopes: 432 assignments and 122 negative assignments. The constructor-heritage oracle has 44 cases over both scopes, 176 inventory signatures, 30 selected origins/results/calls, 48 assignments and 12 diagnostic negatives; `generic_rest_constraint_retained_gap` is supported.

The selected Rust cohort passes 2,161 tests with 32 existing ignored probes, and all seven integrations pass. The file-budget and ID-discipline audits pass. Frozen Query Core is unchanged at every occurrence, with exact fresh/cold parity and zero snapshot changes. Both complete reports equal their epoch-76 predecessors after removing only `elapsed_ms`:

- `resolution-documents/2026-09-09-query-core-generic-array-rest-program-report.json`: SHA256 `67de5e0ac0d94cf83747de269c993933ece57e197fdf014676b2b8c2e12946b0`; 38,808 ms; 585 correct, 40 declaration-kind disagreements and 65 unresolved of 690.
- `resolution-documents/2026-09-09-query-core-generic-array-rest-legacy-report.json`: SHA256 `67b079059e4b8f08e65ae2d958be3a2e35417f225cf061619807193bcd82f969`; 35,498 ms; 557 correct, 40 declaration-kind disagreements and 93 unresolved of 690.

## Contextual generic finite-tuple rest inference closeout

TypeScript 5.9.3 infers a direct generic source rest constrained by a finite tuple or union of tuples from the target's complete remainder. The inferred remainder keeps required and optional tuple slots, readonly structure and union alternatives. The source signature's minimum argument count remains its original fixed prefix; required elements in the generic rest constraint do not become fixed source parameters.

The callable evaluator now canonicalizes generic-rest constraints and defaults without erasing readonly or optional-slot evidence, then applies the same contextual tail inference used for configured arrays to finite tuple and tuple-union domains. A parameter-tail candidate is accepted only when it satisfies the complete source tuple domain. When an inferred empty tail violates a required tuple constraint, a valid correlated return candidate can replace it before declared-constraint fallback. Finite tuple result comparison now checks concrete tuple shapes through the ordinary value relation, so `[number]` cannot satisfy `[]` while legal constrained return correlation remains provable. Source-owned generic IDs, target rigidity, constructor origins and ordinary arity/variance checks remain intact.

Binding epoch is 78; extractor schema remains 83 because serialized extraction facts did not change. The TypeScript 5.9.3 callable oracle has 110 cases: 68 positives, 38 diagnostic-backed negatives, four invalid-source barriers and four legal abstentions. The cross-kind rest oracle has 87 cases over call, construct and method signatures in script and module scopes: 522 assignments and 162 negative assignments. The constructor-heritage oracle has 45 cases over both scopes, 180 inventory signatures, 30 selected origins, 30 exact results, 50 assignments, 30 exact downstream calls and 12 diagnostic negatives; `generic_tuple_rest_constraint` is supported.

The selected Rust cohort passes 2,161 tests with 32 existing ignored probes and 5,622 filtered tests. Both focused compiler-evidence Rust tests pass, as do all seven integrations across the TypeScript, JavaScript and Rust resolution corpora, per-file manifests and per-package context. Native whole-worktree gates report zero file-budget violations across 485 changed production files and zero increased string-identity patterns across 169 scoped resolver/type files. The five callable modules are 138–361 lines, scoped `rustfmt --check` passes and `git diff --check` is clean.

Frozen Query Core remains unchanged at every occurrence. The configured report has 585 correct, 40 declaration-kind disagreements and 65 unresolved labels; the legacy report has 557 correct, 40 kind disagreements and 93 unresolved. Both modes have exact fresh/cold parity and zero snapshot changes, and both complete reports equal their epoch-77 predecessors after removing only `elapsed_ms`:

- `resolution-documents/2026-09-09-query-core-generic-tuple-rest-program-report.json`: SHA256 `8b1512bf24c8cc1b1a59732254d44b834079a1753094e381da4d23786ee4ffd4`; 47,368 ms.
- `resolution-documents/2026-09-09-query-core-generic-tuple-rest-legacy-report.json`: SHA256 `0d4b986b42e0af5510d1c375155928f02a27354c36d3f855fbcfd2e976f0c13d`; 43,046 ms.
- `resolution-documents/2026-09-09-query-core-generic-tuple-rest-comparison.json` records the complete before/after hashes, counts and field comparison for both modes.

The full construct-signature variance parent remains open for dependent tuple constraints, mixed fixed-plus-array target tails, unconstrained generic targets, conflicting array-rest return candidates, call facets and inference from constructor facets.
