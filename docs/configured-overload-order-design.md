# Source-owned overload ordering

1. Generic engine: retain signature parent-group spans and canonical declaring-owner IDs through nominal and inherited surfaces.
2. Profile data: select merged-group/literal-first overload ordering and capture literal annotation syntax; aliases and parenthesized literals are not specialized by value.
3. Configuration boundary: capture compiler source binding order separately from sorted manifest file IDs; absent or invalid order cannot decide cross-file competing groups.
4. Generic engine: reorder source signatures, never physical navigation rows, preserving all candidate origins.
5. Generic engine: try subtype applicability before assignment applicability, with private candidate inference and no shared callback writes.
6. Generic engine: an unknown earlier candidate blocks selection; an unknown subtype candidate blocks assignment-phase fallback.
7. Relations: propagate the phase through nested callable operands, unions and supported structural properties; keep declaration identity distinct from assignability.
8. Existing merge/conditional proof barriers remain unchanged; unsupported variadics, higher-rank instantiation and incomplete sources remain unknown.
9. Verification: failing retained predicate first; independent compiler labels for grouping, literal syntax, subtype ordering, unknown barriers, portable/cold and configuration changes.
10. Measurement: preserve the frozen 575/690 cohort and labels; publish new reports, retain negatives, audit ID/file budgets, leave the multilingual/product goal open.

```ts
interface API<T> {
  pick<S extends T>(f: (x: T) => x is S): S | undefined;
  pick(f: (x: T) => unknown): T | undefined;
}
api.pick(x => typeof x === "string");
// ❌ Before [generic]: body inference succeeds; competing applicability abstains.
// ⚠️ Planned [generic + profile data]: source-ordered subtype selection retains the predicate signature and narrowed yield.
```

Primary reference inspected: local TypeScript 5.9.3 `typescript.js`, `reorderCandidates`,
`resolveCall`/`chooseOverload`, `isSimpleTypeRelatedTo`, `compareSignaturesRelated`,
and `propertiesRelatedTo`. The existing compiler manifest alphabetically sorts
source files, so its file IDs alone are explicitly not source binding-order evidence.

## Implemented evidence

Epoch 62 / extraction schema 77. `SignatureOrder` records parent span, typed
ordering policy and literal-annotation syntax. Canonical declaring-owner IDs are
preserved through inherited member facts. `program_overload_order` orders source
groups and keeps all navigation origins, including rowless signatures.

`Program.source_binding_order` is optional caller-attested configuration. The
compiler adapter records its original source sequence independently of sorted
manifest IDs. Invalid/missing/duplicate/foreign source-order entries cannot order
cross-file competing groups. Configuration-only changes rebuild the view; old
manifests still work but cannot supply ordering evidence they never captured.

`ArgumentRelation` separates subtype and assignment applicability. Subtype proof
propagates through supported callable operands, scalar unions and non-fresh named
object properties. Callable validity still checks defaults/constraints/predicates
using assignment rules. Existing merge/conditional equality proofs retain their
erased/incomplete callable barriers. Earlier unknown candidates block selection;
later unsupported candidates cannot undo an earlier proved selection. Without
ordering evidence, the former unique-applicability proof remains available.

```ts
api.pick(value => typeof value === "string");
// ✅ [generic] Candidate-local predicate inference + source ordering selects the narrowed yield.
api.pick(callbackReturningAny);
// ✅ [generic] Subtype phase inspects nested callable results before assignment fallback.
api.pick(valueWithOptionalProperties);
// ⚠️ [generic] Non-fresh property evidence is supported; fresh object-literal provenance is not inferred.
api.pick(...spreadValues);
// ❌ [generic] Complete variadic/receiver/contextual-inference evidence remains required.
```

Pinned compiler verification: 45 cases, 41 selected-signature/exact-result labels,
four diagnostic-backed negatives and two retained legal receiver/variadic
abstentions. The original ordered-competing, optional-tuple-rest and inferred
predicate labels are unchanged and now supported. Twelve additional single-file
cases and three cross-file cases cover precedence and subtype/assignment phases.
The compiler-capture adapter's three tests pass, including dependency-first source
binding order versus alphabetically sorted file IDs.

Selected library verification: 2,117 passed, zero failures, 31 ignored (34.89 s).
Both local and inline callback-result chains now test competing applicable
overloads, with nullable and nonnullable results and poisoned display arguments.
Portable caches remap arenas; cross-file tests preserve exact declaration origins
and configuration-only reorder/revocation through cold reload.

The first cross-file portable fixture used a synthetic integer literal domain
against a TypeScript source-number literal domain. The engine correctly abstained
on unsupported cross-domain equality. The fixture now resolves its actual argument
through the production source occurrence API; no numeric equality proof was relaxed.

Whole-worktree native audit: 332 changed production files, zero budget violations;
127 resolver/type-system files, zero increased string-identity patterns. Git diff
whitespace validation passes. Node child-process spawning could not run the native
audit, so the same checks were performed with PowerShell and direct Git reads.

Final review caught a failing-before precision regression: an invalid callable
nested inside a union could satisfy a broad subtype target before its operands
were validated. `subtype_input` now checks complete nested callable evidence before
identity/top shortcuts, covering union, tuple, object/operator and application
operands without weakening the separate merge-proof barrier. Two compiler-backed
diagnostic negatives (2677) cover invalid union/property callables against broad
overloads. Final compiler coverage is 47 cases, 41 selected-signature/result labels,
six diagnostic-backed negatives and two legal abstentions. Final selected library
verification: 2,118 passed, zero failures, 31 ignored (34.99 seconds).

The first real-project report predates this final validity guard and is retained
as intermediate evidence: `2026-09-08-query-core-overload-order-program-report.json`.
Its entire JSON is identical to the epoch-61 configured report except elapsed time
(53,023 ms). All 690 labels, observations, and fresh/cold outcomes are unchanged:
575 correct, 40 kind-only disagreements, 75 unresolved. It is not evidence of a
real-project recall gain or a controlled performance benchmark.

## Final measurement and handoff

All seven targeted integrations pass (`resolution_corpus`, `resolution_corpus_js`,
`resolution_corpus_rust`, `per_file_manifest`, `per_package_context`). Final configured
and legacy reports are each wholly identical to the respective epoch-61 reports
except elapsed time. All 690 labelled fresh/cold observations match exactly.

- Configured: `2026-09-08-query-core-overload-order-verified-program-report.json`,
  SHA-256 `94c3af143e441530a770d44e8b78059f5601e5fa393fdc8fee34342ec57300a1`,
  elapsed 53,143 ms; 575 correct, 40 kind-only disagreements, 75 unresolved.
- Legacy: `2026-09-08-query-core-overload-order-verified-legacy-report.json`,
  SHA-256 `9b4017adf4c4040131e51a4daa49d6b97c4cc3a785d6fd6c428e035ff8efe352`,
  elapsed 61,247 ms; 557 correct, 40 kind-only disagreements, 93 unresolved.
- Fresh diagnostic: `2026-09-08-query-core-overload-order-trace.json`,
  SHA-256 `a55e2c027a3d88a17267b77f9b2368795db9cb73a6fe94b31af9743abe21c6c0`,
  elapsed 51,330 ms; 75 requested sites, 264 records, zero baseline/snapshot changes.
- Intermediate configured report SHA-256:
  `42c3682ed88eb1696af6a9e573a5f5abd7c3013ad153dfb662c55f02ff13dcf8`.
- Fixed manifest remains SHA-256
  `44158824a3f91fdeda2d5e5da8a422054e178c76de29857c2f855ffe36a5110c`.

Timings are diagnostic, not controlled benchmarks; configured and legacy runs
overlapped in separate processes. No existing report or source label was overwritten.
The 99% correctness gate remains false.

```ts
const excludeSet = new Set(array2);
return array1.filter(x => !excludeSet.has(x));
// ❌ [generic] Fresh trace retains the Set initializer/receiver and callback cascade.
// Constructor generic inference and negated body evidence need tracing/proof;
// ordering alone did not close these sites.

export const environmentManager = (() => {
  let isServerFn: IsServerValue = () => isServer;
  return { isServer(): boolean { return isServerFn(); } };
})();
environmentManager.isServer();
// ❌ [generic] Fresh trace still has Unknown at the imported receiver root.
// Next foundation: IIFE return/object member origins, not an isServer name hook.

const originalQueue = queue;
originalQueue.forEach(callback);
// ❌ [generic] Fresh trace still reports an untyped local binding in notifyManager.
// Keep assignment/value publication separate from overload ranking.
```

`Program.source_binding_order` is a public configuration-field addition. Read-only
consumer searches found no matching construction sites in AlphaT's source or Lynx's
impact module. Fresh serial `cargo check --offline --locked` checks still stop before
source verification: AlphaT hits the yanked `der 0.8.0` dependency through
`ort 2.0.0-rc.12` / `ureq 3.3.0`; Lynx requires a lockfile update. No sibling sources
or lockfiles were changed. Their previously recorded lock hashes remain unchanged.
Full downstream compilation is therefore unverified, not claimed successful.

Only the supported ordered-selection child is checked in the roadmap. Full
callable/receiver/variadic and initializer semantics, multilingual ID migration,
representative compiler correctness gates, IDE occurrence APIs, AI benchmarks and
cross-service flows remain required by the active goal.
