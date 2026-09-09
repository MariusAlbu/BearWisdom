# Source-owned callable type identities

1. Generic engine work: add a complete callable type alongside the explicitly incomplete legacy Function representation.
2. Own callable identity by configured-program context, source instance and exact source signature span; no physical row or display name is required.
3. Carry parameter source slots, optional/rest/receiver flags, locally owned generic IDs, constraints/defaults and predicate target slots as typed data.
4. Capture function-type syntax through profile data, lowering nested signatures recursively without formatting and re-resolving names.
5. Convert predicate parameter names to signature-local source identities at ingestion; reject missing/duplicate/destructured targets.
6. Keep portable source recipes independent of runtime arenas and allocate selected-source generic identities before materialization.
7. Remap all callable operands and refresh origin contexts on snapshot hydration; reject foreign/stale contexts even for scalar-only signatures.
8. Substitute free generic IDs while protecting the callable's own binders; keep source origins separate from structural assignability.
9. Retain incomplete applicability barriers until full source-owned callable relations and candidate-local callback inference are implemented.
10. Verify compiler-backed source shapes, nested generic shadowing, predicate targets, poisoned/rowless/portable/cold data and provider changes, then remeasure the unchanged real-project cohort.

## Implemented source evidence

```ts
type Callback = <T extends string = string>(value: T, flag?: boolean) => value is T;
// ✅ [profile data + generic] Function-type recipes retain source signature
// origin, local generic identity, constraint/default, parameter source slots,
// optional/rest/receiver axes, and the exact predicate target slot.

type Nested = <T>(outer: T) => <T extends number>(inner: T) => T;
// ✅ [generic] Distinct source generic IDs; substitution protects the inner
// callable's own binders while rewriting free outer parameters.

values.find(value => typeof value === 'string');
// ❌ [generic] Candidate-local callback body inference and predicate inference
// are not implemented by source capture alone; applicability still abstains.
```

Binding epoch 59 / extractor schema 75. `Type::Callable` is a separate source-owned representation; the legacy `Function` arm remains explicitly incomplete. Callable origins include configured context, source instance and signature span, including scalar-only signatures and those without navigation rows. Predicate names are interned and converted to matching signature-local source parameter slots during ingestion; runtime types contain no name-based predicate lookup. Missing/duplicate predicate targets mark the callable incomplete.

`Callable::map` and operand traversal include generic binders/constraints/defaults, parameter values, return values and predicate types. Arena snapshots refresh callable contexts alongside nominal/unique contexts; merges remap every operand. Content-keyed parse caches do not revive runtime callable handles: normal source contract restoration reconstructs portable recipes and selected-source identities. Legacy name requalification and generic-name rebinding cannot rewrite source-owned callables.

Configured recipes and lexical annotations materialize this evidence using the selected source's signature inventory. Unconfigured import-dependent recipes retain their prior legacy representation. Contextual parameter seeding can read supported callable parameter types without recovering display text; full applicability continues to reject callable evidence until the corresponding relation is proved.

## Verification and next boundary

- The pre-change configured predicate probe failed because its type was an erased `Function` with an unresolved result. It now retains the exact generic and predicate identities.
- Independent TypeScript 5.9.3 verifier: seven cases, eight signature source positions, eleven parameter positions, three predicate targets, one diagnostic-backed negative.
- Engine tests cover every fixture fresh/cold, portable caches with shifted arenas, poisoned symbol display metadata, rowless nested signatures, generic shadowing/free substitution, source identity isolation, provider retarget/edit/deletion, and stale-context rejection.
- Selected library suite: 2,096 passed, 31 ignored, zero failures. Existing eighteen-case overload compiler verifier remains unchanged and passing, including its six legal abstentions.

The next implementation must supply callable relations and source-addressed callback bodies to candidate-local binding environments. `CallArg::LambdaAt` currently retains parameter spans but no whole-body identity/return recipe, and `arg_types` produces Unknown for it. `chain_bound_method::call` types arguments before overload selection; candidate evaluation must obtain the source callback, infer its return/predicate under each candidate context, and publish contextual writes only after selection. Merely reading `Callable.parameters` after selecting an overload cannot solve that dependency.

Callable source identity is not structural assignment equality: independently declared but compatible callable types have different origins. Compare full signatures with generic alpha-binding, variance, arity and predicate obligations; never collapse origins to force TypeId equality. Standalone function values, method callable projections, complete receiver/variadic/access rules and language ordering remain separate migration work. AlphaT/Lynx consumer compilation remains unverified owing to the previously recorded dependency/lockfile incompatibility; no sibling lockfiles were changed.

## Real-project measurements

Both complete reports match the final epoch-58 overload-evidence reports except elapsed time; every fresh/cold labelled occurrence is identical. Configured Query Core remains 571 correct / 40 declaration-kind-only disagreements / 79 unresolved of 690 labels (82.7536231884% recall; 93.4533551555% strict precision). Legacy remains 557 / 40 / 93. The fixed manifest still has 843 compiler calls, 690 labels and 153 unlabelled calls; this development cohort is not representative 99% evidence, and the gate remains false.

- Configured: `resolution-documents/2026-09-08-query-core-callable-identities-program-report.json`, SHA-256 `2985942f4665ed1d0e2439e83be7f9d97db08a81006835b7ca3cbe0c992f49f6`, elapsed 50,508 ms.
- Legacy: `resolution-documents/2026-09-08-query-core-callable-identities-legacy-report.json`, SHA-256 `dd85e61a73d8915cb5b753b755fe5096a148b564df05ddc40ba9708959b3ebae`, elapsed 67,271 ms (overlapped library compilation; not a controlled benchmark).

Final full selected-library rerun: 2,096 passed, 31 ignored, zero failures in 29.88 seconds, including the additional syntax-axis assertions. Native audits: 327 changed production files, zero file-budget violations; 123 resolver/type-system production files, zero increased string-identity patterns. `git diff --check` passed.

All seven targeted integrations passed: `resolution_corpus`, `resolution_corpus_js`, `resolution_corpus_rust`, `per_file_manifest` and `per_package_context` (the latter two each contain two tests).

Consumer compilation was rechecked serially with `cargo check --offline --locked`, each explicit manifest and the BearWisdom target directory. AlphaT failed dependency selection before compilation: `ort =2.0.0-rc.12` → `ureq ^3.1` → yanked `der 0.8.0` in the available offline index. Lynx requires a lockfile update and `--locked` refused it. Neither check verifies source compatibility. Both lockfile hashes are unchanged: AlphaT `27dd905fb100ac46634f45ad0e61fa3a1332590a7ccef90373e2682b25b8699a`; Lynx `054bb8595f56083cb7b311b7ae36fd9200c7e9bd1923a21a6243f20eeaf9fadc`. No verification processes remain running.

Epoch-60 follow-up: [configured callable relations](configured-callable-relations-design.md) implements supported argument applicability without collapsing source origins. Candidate-local callback bodies and ordered overload selection remain the next boundary; the final real-project cohort remains unchanged.
