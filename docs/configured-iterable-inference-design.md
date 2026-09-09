# Configured structural argument inference

1. [generic] Infer across distinct nominal heads through selected source member keys and full callable signatures, preserving declaration origins and GenericParamIds.
2. [generic] Reuse nominal member substitution and ancestry inventories; expose complete comparison evidence rather than erasing methods into Function values.
3. [generic] Retain hidden call/construct facets as explicit completeness barriers until their independent signatures are compared.
4. [generic] Compare required/optional properties and every target method overload using the existing policy-aware callable relation; preserve subtype versus assignment phases.
5. [generic] Keep recursive structural pairs bounded and candidate-local; unknown or invalid operands cannot create success through a cycle or namesake.
6. [generic] Compare finite union-of-tuple rest signatures as correlated tuple alternatives, retaining arity and optional slots instead of flattening their union to independent parameters.
7. [generic] Separate inference candidate collection from final applicability proof, and retain unsupported higher-rank, index and variadic evidence as unknown.
8. [generic] Capture independent TypeScript 5.9.3 constructor result/signature and downstream call labels, plus diagnostic-negative member, optional, rest and recursive cases.
9. [generic] Verify poisoned portable/cold inputs, selected-provider/configuration changes, edits/deletion, unrelated source namesakes and incomplete signature facets.
10. [generic] Recheck the real Set(array2) initializer and frozen Query Core cohort; exact compiler constructor order and callback negation remain separate dependencies until verified.

## Implementation and evidence

Binding epoch 68 / extractor schema 80. Source member keys and complete callable signatures now supply cross-head inference candidates. A separate argument proof checks optional/required membership, callable-property variance, and every target overload. Recursive pair hypotheses remain local to one bounded proof and are never stored as independent successes. Call/construct facets excluded from keyof inventories now carry an explicit structural-comparison barrier, including inherited facets.

The [TypeScript compatibility documentation](https://www.typescriptlang.org/docs/handbook/type-compatibility.html) describes recursive member comparison for function arguments. The pinned 5.9.3 compiler's inferFromProperties/inferFromSignatures and compareSignaturesRelated routines provide the reference behavior for the independent fixtures. Finite union-of-tuple rest signatures preserve their whole tuple alternatives and optional slots; parameter-wise unions would incorrectly admit swapped correlated arguments.

The compiler verifier checks 21 fixtures in script and module scopes: 24 exact constructor-result type labels, 24 constructor signature declaration labels, four downstream call declaration/signature labels, and nine diagnostic-negative cases. The engine checks the exact constructor field type and downstream targets after poisoned display metadata, shifted portable extraction arenas and cold reload. A second regression checks configuration-only provider/policy changes, foreign contexts, source edits, stale hashes, deletion and recovery of unchanged consumers.

The selected library suite passed 2,144 tests (32 ignored). The initial regression reproduced missing cross-head property inference. The broader suite caught a class callback regression introduced by attempting member expansion before its existing identity proof; known nominal identity/constraint evidence now precedes structural expansion, and the compiler-labelled overload case passes again.

```ts
new Build(input);             // [generic] passes property/method/recursive/iterator inference fixtures
holder.value.read().touch();  // [generic] passes both exact downstream source targets, fresh and cold
new Set(array2);              // [generic] real retained initializer now yields Set<caller T>
```

The real Query Core provenance probe confirms both selected constructor candidates now apply and produce the same Set<caller T> TypeId: the readonly-array declaration in lib.es2015.collection.d.ts and the iterable declaration in lib.es2015.iterable.d.ts. ArrayIterator and both IteratorObject providers remain admitted; the global intrinsic and Node namespace alias identities remain distinct. This proves the initializer result, not the exact selected constructor signature: the old manifest lacks compiler binding order, and the constructor helper currently requires applicable candidates to agree on the result.

Remaining work includes exact constructor ordering/selection and callback negation in difference<T>. General index inference, higher-rank/ordered overload inference, non-finite variadic rests and call/construct structural facets remain incomplete evidence. Anonymous object/member origins, conditional member-receiver aliases and the environmentManager IIFE cascade remain separate roadmap work.

All seven targeted integrations passed: TypeScript, JavaScript and Rust resolution corpora, plus per-file manifest and per-package context tests. The native Windows audit found no budget violations or new string-identity patterns across 343 changed production files and 135 resolver/type files. Git diff whitespace checks passed. This step changes internal engine evidence; the previously recorded AlphaT/Lynx dependency and lockfile compilation limitations remain unresolved.

## Frozen cohort results

The original manifest remains unchanged: `resolution-documents/2026-09-07-query-core-configured-compiler-manifest.json`, SHA-256 `44158824a3f91fdeda2d5e5da8a422054e178c76de29857c2f855ffe36a5110c` (187 supplied sources, 24 selected files, 843 compiler calls, 690 labelled targets, zero compiler diagnostics).

- Configured: `resolution-documents/2026-09-09-query-core-iterable-inference-program-report.json`, SHA-256 `16baac1732677df5ec75eec5ad07b9de64272e16fd2eaff31e01dc73f755893d`, elapsed 48,222 ms. Fresh/cold: 575 correct, 40 kind-only incorrect, 75 unresolved, 83.3333% recall, 93.4959% strict precision, zero snapshot changes, gate ineligible.
- Legacy: `resolution-documents/2026-09-09-query-core-iterable-inference-legacy-report.json`, SHA-256 `cffae670828299a00e6eeff7e2f16020064b16401ce473403714f08da894fc3e`, elapsed 46,216 ms. Fresh/cold: 557 correct, 40 kind-only incorrect, 93 unresolved, zero snapshot changes, gate ineligible.

Deep equality excluding only elapsed_ms confirms both reports are unchanged at every field and retained occurrence from the method-heritage reports. The real initializer is now typed, but the dependent filter callback still needs source-owned negation support. Existing roadmap lines were preserved and one completed child was appended; broader parents remain open.

The next callback change must preserve predicate inference, not merely yield boolean. A pinned compiler probe of `value => !value` infers a predicate selecting undefined for `object | undefined` and `true | undefined`; it returns an ordinary boolean for boolean, string, number, string | undefined, string | number, unknown, any and never. Nested negated typeof tests must also preserve their two-sided narrowing. Operand member access, private/missing members and unsupported callable predicates must still be validated before a callback contributes applicability evidence.
