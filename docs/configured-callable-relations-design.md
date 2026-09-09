# Source-owned callable applicability

1. Extend the generic configured-program relation, not member-name hooks or display-based function matching.
2. Preserve both callable origins; alpha-bind signature-local generic IDs only inside a private comparison environment.
3. Carry parameter-variance and nullability policy as typed configured-program inputs; absent policy is unknown, not an assumed compiler option.
4. Read compiler options only at the oracle ingestion boundary and persist the typed policy with source/configuration membership.
5. Compare receiver, required/optional/rest arity, contravariant parameters and covariant returns using one bounded evaluation context.
6. Match predicate targets by source parameter position and distinguish predicate kinds; declaration spelling never participates.
7. Retain fixed tuple-rest optional slots during canonicalization; unsupported variadic evidence must remain unknown.
8. Keep the legacy erased Function proof barrier and require complete source-owned evidence before callable assignment.
9. Validate independent compiler assignment/selected-signature labels, negatives, distinct origins, portable/cold restoration and configuration changes.
10. Feed these proofs into overload applicability, then continue source-owned candidate-local callback body inference and remeasure the fixed cohort without changing its labels.

This is the callable-relation dependency of candidate-local callback inference. It does not, by itself, capture or evaluate callback bodies or implement language overload ordering.

## Implemented behavior

```ts
type Left = <T>(value: T) => T;
type Right = <U>(item: U) => U;
// ✅ [generic] program_callable_relations alpha-binds GenericParamIds privately;
// both configured source/signature origins remain distinct.

declare const callback: () => number;
// Factory overloads accept () => string or () => number and yield distinct owners.
const result = api.pick(callback);
result.number();
api.pick(callback).number();
// ✅ [generic] callable argument proof selects the numeric callback overload;
// both local and inline result chains retain exact declaration targets.

type A = (callback?: (value: string | number) => void) => void;
type B = (callback?: (value: string) => void) => void;
// ✅ [generic + configured policy] A cannot substitute for B even with relaxed
// ordinary function variance: nested callback parameter rules remain distinct.

values.find(value => typeof value === 'string');
// ❌ [generic] source-owned callback body/predicate inference and ordered
// candidate selection remain open; declared callable evidence is not a body.
```

Binding epoch 60; extraction schema remains 75 because source callable capture did not change. `Program.callable_policy` is an optional typed configuration input with explicit parameter-variance and nullability rules. Old serialized configurations restore `None`. The real-project oracle consumes compiler option spelling only at ingestion, including explicit overrides of `strict`, and persists the resulting policy with the selected configuration. Other callers must supply attested policy; absence does not authorize guessing.

The bounded `Eval` now owns private constraint overrides and rigid generic IDs. Overload applicability supplies receiver-substituted method constraints before checking callback shape; those constraints are never written back to shared program facts. This is necessary for an `S extends T` predicate candidate after the receiver's `T` has become a concrete union. Generic upper-bound proof precedes union-arm distribution. Supported declared callable parameter/return/predicate operands can infer method generic arguments; local higher-rank inference remains fenced.

Callable comparison covers distinct origins, matching alpha-bound generic domains, optional and fixed tuple-rest arity, receiver parameters, ordinary versus nested callback variance, nullable callback wrappers, return covariance/ignored returns and source-slot predicate targets. Invalid predicate/default evidence, erased `Function`, incomplete source types, foreign snapshots and exhausted proof budgets do not become successful comparisons. Fixed tuple-rest optional markers survive canonicalization instead of becoming required union-valued slots.

The new path serves argument applicability. It does not globally remove the callable barrier from conditional/merge equality proofs. Different generic constraint domains, generic-to-monomorphic contextual instantiation, complete variadic tuples, instantiated-generic callback provenance, standalone values/method projections, full invalid-source diagnostics and class-method/access/ordering semantics remain explicit work. Callback bodies still need whole-source identities, return/narrowing recipes and candidate-local BindingId environments before any contextual writes are published.

## Verification

The original regression returned `None` for two independently declared compatible signatures. Compiler-backed tests now exercise assignment, selected signature and downstream result behavior, portable contract caches with shifted arenas and poisoned display metadata, fresh/cold source origins, configuration-only policy changes, stale-context rejection and bounded/erased/incomplete barriers. Existing ordinary-arrow and inferred-predicate overload probes retain their original labels and unsupported status.

Return labels are compared after canonicalizing the source-owned expected type, since a nested union annotation and the equivalent flattened inferred union can have different arena handles. Compiler target/result labels were not altered.

Final review added a failing-before regression for invalid callable evidence against a broad `unknown` target. Signature-to-signature validation alone was insufficient: a broad target bypassed it and returned `Some(true)`. Shared source-validity checks now guard those paths too. The first `callable-relations-{program,legacy}-report.json` files predate this final guard; final verified reports are retained separately, never overwritten.

Final selected library verification: 2,103 passed, zero failures, 31 ignored (31.00 seconds). All seven targeted integrations passed: `resolution_corpus`, `resolution_corpus_js`, `resolution_corpus_rust`, `per_file_manifest` and `per_package_context` (two tests in each of the latter two).

Pinned TypeScript 5.9.3 callable verifier: 51 cases, 29 legal positive assignments, 19 diagnostic-backed assignment negatives and three invalid-source barriers. Three of the legal positives remain engine abstentions. The separate overload verifier passes 23 cases / 20 independently selected-signature and exact-result labels / three diagnostic negatives; six legal cases, including both inline-arrow probes, remain engine abstentions. Four new supported declared-callable selections and one new negative were added without changing the previous eighteen labels.

Native whole-worktree audit: 328 changed production files, zero file-budget violations; 124 scoped resolver/type-system files, zero increased string-identity patterns. `git diff --check` passed. No workspace-wide, lint or formatting commands were run.

## Final real-project evidence and downstream boundary

Both final reports are wholly identical to the epoch-59 callable-identity reports except elapsed time. Each report contains 690 reference verdicts, and fresh/cold arrays match exactly. Configured Query Core remains 571 correct / 40 declaration-kind-only disagreements / 79 unresolved (82.7536231884% recall; 93.4533551555% strict precision). Legacy remains 557 / 40 / 93. No labelled occurrence improved or regressed; the 99% gate remains false.

- Configured: `resolution-documents/2026-09-08-query-core-callable-relations-verified-program-report.json`, SHA-256 `bc6cb88c169cf8526cb9097a86561db24550ed9334dffcef5be2a776df5e0a00`, elapsed 45,270 ms.
- Legacy: `resolution-documents/2026-09-08-query-core-callable-relations-verified-legacy-report.json`, SHA-256 `4965703e3eb40547fe81dce75a52671ae7e50d26f669e9b513bd3813bcf9798a`, elapsed 49,174 ms. Timings are diagnostic runs, not controlled product benchmarks.
- The fixed 843-call/690-label/153-unlabelled compiler manifest is unchanged: SHA-256 `44158824a3f91fdeda2d5e5da8a422054e178c76de29857c2f855ffe36a5110c`.

The pre-final-guard reports are also retained: configured SHA-256 `cb498d793e28728118587c34779eacce3cdbc8a57992052a8373b01e78d63fc7` (45,627 ms), legacy `01d0d56f864a18cbb1da7700abef208b87cbd9c74b15aa51d3d4847f783f6baf` (48,590 ms). Those too match the predecessor reports except elapsed time.

Serial `cargo check --offline --locked` downstream checks remain blocked before source compatibility is verified. AlphaT fails dependency selection at `ort =2.0.0-rc.12` → `ureq 3.3.0` → yanked `der 0.8.0` in the offline index. Lynx requires a lockfile update that `--locked` refuses. Explicit manifests and the BearWisdom target directory were used; no sibling sources or lockfiles were edited. Lock hashes remain AlphaT `27dd905fb100ac46634f45ad0e61fa3a1332590a7ccef90373e2682b25b8699a` and Lynx `054bb8595f56083cb7b311b7ae36fd9200c7e9bd1923a21a6243f20eeaf9fadc`.

Only the bounded callable-relation prerequisite is checked in the roadmap. The parent callback/overload task, representative multilingual correctness gate, snapshot/IDE integration, comparative AI benchmarks and cross-service flows remain open.
