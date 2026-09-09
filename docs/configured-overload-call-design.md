# Source-owned overloaded calls

1. Generic engine work: selected-program callable groups, complete source signatures and argument applicability; no library-name hook or resolver string recovery.
2. Group methods by canonical receiver owner and MemberNameId from complete proved nominal surfaces, retaining rowless source origins and every overload candidate.
3. Keep group navigation origins separate from the selected signature; never canonicalize overload rows together or use physical row order as dispatch evidence.
4. Substitute receiver GenericParamIds through full parameter, result, default and constraint recipes, including effective inherited signatures.
5. Share constructor argument applicability with method calls, retaining tri-state incomplete/inapplicable/applicable evidence and fixed-rest arity.
6. Initially select only a uniquely proved applicable signature; preserve legal multiple-applicable ordering cases and contextual callback competitors as explicit pending semantics.
7. Feed the selected instantiated signature into chain traversal and contextual writes, bypassing legacy name-based overload/yield retries.
8. Capture independent compiler selected-signature source addresses and exact return-type identities separately from call-symbol navigation labels.
9. Verify fresh/cold, generic owners, source retarget/edit/deletion, rowless and poisoned display metadata, and ambiguous/incomplete competitors without weakening existing gates.
10. Remeasure all retained Query Core occurrences against 571/690; then extend ordered/contextual dispatch and general initializer/return propagation toward the full 99% target.

## Implemented and verified

```ts
interface Factory {
  pick(value: string): TextResult;
  pick(value: number): NumberResult;
}
const result = factory.pick(42);
result.number();
factory.pick(42).number();
// ✅ [generic] Complete nominal source group + argument TypeIds selects the
// numeric signature and feeds its result through both local and inline chains.
// The other overload's source origin remains intact; no row canonicalization.

interface ReceiverSensitive {
  pick(this: ReceiverSensitive, value: number): string;
  pick(value: number): number;
}
receiver.pick(42);
// ⚠️ [profile data + generic] The receiver parameter is captured separately.
// Without receiver-constraint/ordering proof, abstain rather than count it as
// an ordinary argument and incorrectly discard the first signature.

values.find(value => typeof value === 'string');
// ❌ [generic] Source callable/predicate identities and contextual inference
// are still missing; an erased Function type is not applicability proof.
```

Binding epoch 58 / extraction schema 74. `lexical_call_arguments` captures configured TS/JS call operands independently of extracted-reference display payloads, including exact source literal atoms, identifier bindings and unsupported/duplicate-selector barriers. Shadowable `undefined` is a value read, not an assumed atom. Profile data identifies call/selector/receiver syntax; there are no library-specific hooks. Unconfigured consumers retain their existing argument path.

`program_overload_calls` gathers full proved nominal method surfaces by canonical owner and MemberNameId. Every candidate retains SourceInstanceId, signature span and optional physical navigation row. Effective inherited signatures are substituted with the receiver's GenericParamIds. The shared `program_constructor_arguments` evaluator now returns instantiated parameter/result types as well as applicability; a unique applicable signature can feed chain traversal without legacy overload-yield retries. Multiple applicable signatures, unknown competitors, optional methods and incomplete groups remain barriers. Direct generic return positions preserve literal inference; container-result inference retains supported widening.

Source signatures now retain explicit receiver-parameter flags. Fixed tuple rests preserve optional element arity; unknown tuple/rest shapes cannot be interpreted as a single required slot. A compiler-labelled receiver-parameter probe failed with a fabricated overload before this correction. Full receiver-constraint and ordered dispatch remain pending rather than being approximated.

Verification:

- Pinned TypeScript 5.9.3 verifier: 18 cases, 16 independently selected signature source addresses and exact result identities, two diagnostic-backed negatives, six legal abstentions. Ten cases currently support engine signature selection; all cases are tested fresh/cold.
- End-to-end local and inline result cascades survive poisoned segment names and extracted argument payloads.
- Unchanged consumers follow provider-barrel retargeting and reject inapplicable provider edits/deletion without borrowing namesakes.
- Source overload signatures/origins survive real contract filtering that removes both method rows, shifted-arena portable caches, poisoned symbol display metadata and cold reload. Rowless results are available internally; the existing chain navigation API still requires a physical row.
- Final selected library suite: 2,089 passed, 31 ignored, zero failures. Constructor compiler cohort remains 25 cases / 17 exact types and signatures / seven diagnostic negatives / one legal abstention.

## Remaining work

Full source-owned callable type identities must retain nested signature generic/optional/rest/receiver/type-predicate evidence. `lexical_type_syntax` currently lowers function types into parameter/result vectors, while `program_conditional_eval::proof_value` intentionally rejects that erased Function evidence. Contextual callback evaluation needs candidate-local parameter bindings and return/predicate inference before overload dispatch; selecting a candidate first and only then seeding its lambda parameters cannot solve this dependency.

Language ordering/specificity, class method overload inventories and access, general variadic/named tuples, standalone function overloads, source-span-only IDE navigation and broad program isolation/performance evidence remain open. The retained ordinary-callback and inferred-predicate fixtures provide independent next-step labels. Array/Promise cascades, IIFE/object-return roots and structural callable/intersection origins remain separate upstream causes.

## Final epoch-58 measurements

The final configured report `resolution-documents/2026-09-08-query-core-overload-evidence-program-report.json` retains 571/690 exact matches, 40 declaration-kind-only disagreements and 79 unresolved calls; strict recall 82.7536231884%, precision 93.4533551555%. The final legacy report `2026-09-08-query-core-overload-evidence-legacy-report.json` retains 557 correct, 40 kind disagreements and 93 unresolved. Each complete report equals its constructor-initializer predecessor except elapsed time, and all fresh/cold occurrences match. Neither passes the 99% gate. Seven targeted integrations passed alongside the recorded 2,089 library tests.

Configured SHA-256: `5a6b0b6124c0319a6b630a7a158955e76ce1b28f33a383e66d8e95d028798afc`. Legacy SHA-256: `ffa7e19ae8f541984803d89dec9d508ece51e94fda014335416a28752f6b57ec`. The legacy report's elapsed time overlapped integration compilation and is not a controlled performance benchmark. Later source-callable identity work is recorded separately in `configured-callable-identities-design.md`.
