# ID-based structural evaluation and compatibility

1. Extend the generic type relation behind private merge/inheritance proofs; no language-specific API lists, name recovery or relaxed completeness checks.
2. Retain source mapped-binder, modifier and homomorphic/distribution evidence through substitution before evaluating instantiated types.
3. Evaluate explicit object property/index operands, finite mapped keys, key remaps, keyof and indexed access using TypeIds; no formatted-type lookup.
4. Preserve optional/readonly/index axes, reject invalid key domains and duplicate declarations, and union the value types of collided mapped keys.
5. Treat the empty structural object as the non-nullish type, distinct from the object intrinsic and from engine Unknown.
6. Normalize proven unions/intersections by IDs with bounded recursion; do not erase an unknown structural operand or claim a nominal/structural mixture was evaluated.
7. Compare object surfaces by required/optional presence and value assignability; equality requires both complete canonical shapes, not the same unresolved TypeId.
8. Feed only proven structures into interface base admission. Nominal surface extraction, generic obligations and navigation provenance must stay explicit when combining nominal and structural bases.
9. Start with shared compiler-backed assignment/shape fixtures, then unknown/cycle/depth/foreign-context negatives and source/provider edit/deletion/cold controls.
10. Remeasure the original real-project occurrence population. The two QueryObserverOptions regressions, repeated-property query dependencies, initializer inference and the full F0–F3 goal remain open until independently verified.

## Implementation and evidence — 2026-09-08

`program_structural_eval` now evaluates source-bound aliases with source-owned generic constraints/defaults, declaration-cycle detection, foreign-context rejection and depth/work budgets. `program_structural_ops` evaluates explicit objects, finite string/unique-symbol mapped domains, remaps, supported `keyof` and indexed access. `program_structural_intersection` separates assignment surfaces from declaration identity. These are generic engine operations over IDs, not API-specific hooks or name recovery.

```ts
type A = { a: string; b: number };
type B = { b: number; a: string };
// ✅ [generic] Canonical object keys are sorted by TypeId, not source order.

type C = { a: string } & { b: number };
// ✅ [generic] Relation.assignable(C, A) compares the combined surface.
// ✅ [generic] Relation.equal(C, A) is false: TypeScript rejects a repeated
// property declared once with C and once with A (diagnostic 2717).

type Impossible = { a: 'x' } & { a: 'y' };
// ✅ [generic] Source discriminant evidence reduces the whole type to never.
type HasNever = { a: never } & { a: 'x' };
// ✅ [generic] An explicitly never-valued property does not grant that reduction.

type Copy<T> = { -readonly [P in keyof T]-?: T[P] };
// ✅ [generic] Supported explicit-object instances retain modifier origins.
// ⚠️ [generic] Union/tuple homomorphic distribution remains unsupported.
```

Independent TypeScript 5.9.3 evidence lives in `structural_relation_fixtures.json`, `structural_obligation_fixtures.json` and the expanded `merge_compatibility_fixtures.json`. The assignment verifier checks both exact diagnostics and compiler assignability. The obligation cohort separately distinguishes invalid source types from valid recursive types on which this evaluator abstains. The Rust cohort restores filtered portable inputs into a shifted arena, tests fresh and cold configured views and rebuilds with poisoned display metadata. Additional tests exercise actual barrel retargeting/provider deletion, constraint invalidation, unowned generics, foreign nominal operands, duplicate keys, unknown operands and depth/branch limits.

Three failing-before observations are retained in test design: `reordered_properties` could not be proved; a string index incorrectly satisfied an incompatible numeric index; flattening an intersection into an object incorrectly admitted a compiler-rejected repeated property. The final implementation preserves intersection identity and derives assignment surfaces separately.

### Explicit remaining limits

- Numeric/string property-key equivalence still needs source-attested ID metadata. Numeric literal keys and uncertain string-key/number-index overlaps abstain; evaluation never formats a number into a string and searches for a declaration.
- Homomorphic union/tuple distribution, mixed-modifier remap collisions, broad mapped domains, conditional/infer/template evaluation and recursive structural types remain incomplete. Empty mapped outputs do not erase unknown bodies.
- The current optional-property rules are checked under strict TypeScript with `exactOptionalPropertyTypes: false`; configuration-policy completeness is not claimed.
- Mixed nominal/structural intersections retain all branches but do not yet expose a proved nominal member surface. Structural interface admission and mapped-member navigation provenance still need implementation; the two real QueryObserverOptions regressions are not declared fixed.
- Alias arguments now require available constraint evidence. This is not a complete generic legality, variance or constraint solver. Broader substitution resource bounds and canonicalization coverage remain separate work.
- Repeated-property query dependency staging, initializer-derived fields, representative multilingual 99% evidence, IDE APIs, AI benchmarks and cross-service flows remain open.

Historical baseline for this evaluator step: binding epoch 51 / extraction schema 70. Subsequent nominal queries and supported conditional/infer evaluation are recorded in `nominal-structural-surfaces-design.md` and `source-conditional-inference-design.md` (epoch 53 / schema 71); the broader limits below are not a claim that the newer supported cases remain unimplemented.

### Verification

- Independent compiler checks: 41 assignment cases (26 positive, 15 diagnostic-backed negative); 11 obligation cases (five supported proofs, four invalid source types, two legal recursive abstentions); 25 merge cases (seven positive, fourteen diagnostic-backed negative, four declared unsupported).
- Final selected library verification: 2,050 passed, zero failed, 30 existing ignored tests, 5,623 filtered; 27.60 seconds. This is the selected TS/JS/framework, core type, lexical, resolver, flow, contract/cache and oracle suite, not a workspace-wide test claim.
- All seven selected integrations passed: `resolution_corpus`, `resolution_corpus_js`, `resolution_corpus_rust`, `per_file_manifest` and `per_package_context`.
- Native file-budget audit: 312 changed production sources, zero violations. ID-pattern audit: 114 scoped production sources, zero added violations. `git diff --check` passed. The first native budget attempt incorrectly removed the final newline from HEAD text and reported four one-line false alarms; a symmetric line-array comparison corrected the audit without editing those files.

### Real-project measurement

The configured report `resolution-documents/2026-09-08-query-core-structural-evaluation-program-report.json` is identical to the preceding structural-recipes report after removing only `elapsed_ms`: all 690 fresh and 690 cold labelled occurrences, targets, metadata and other report fields are unchanged. Counts are 546 correct / 40 kind-only disagreements / 104 unresolved; recall 79.1304347826%, strict precision 93.1740614334%, zero source gaps, zero snapshot changes, gate false. Elapsed time was 45,205 ms, an uncontrolled diagnostic runtime rather than comparative performance evidence.

Configured report SHA-256: `b60e722689c3a32ae4e9ada94c06867e27c088b9cc22f0a9f4ac9503c46a26e1`. Unchanged compiler manifest SHA-256: `44158824a3f91fdeda2d5e5da8a422054e178c76de29857c2f855ffe36a5110c`.

This step enables previously unproved structural compatibility and valid rich-property merges in compiler-backed fixtures, but does not yet improve real Query Core binding recall. Both inherited QueryObserverOptions regressions remain present. The next integration must make nominal source-owned member/key surfaces available to `keyof`/mapped constraints and retain navigation provenance, rather than relaxing the nominal-base guard or ignoring a structural operand.

The legacy report `resolution-documents/2026-09-08-query-core-structural-evaluation-legacy-report.json` is also identical to its structural-recipes baseline except elapsed time: 557 correct / 40 kind-only / 93 unresolved, fresh/cold equal, gate false. Elapsed time 52,939 ms; SHA-256 `ba9cdc413f7f717a4caad5c8c01f76600b7533e11c614738d381c7a0d385a947`. Both report processes completed successfully and no Cargo/rustc/oracle process remained at the final check.
