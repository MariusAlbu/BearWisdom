# Candidate-local source callback inference

1. Capture callback bodies at exact call-selector/argument positions in lexical ingestion; retain the existing parameter-span publication API.
2. Express callback, return, operator and primitive-test syntax as profile data, never library/member-name hooks.
3. Inventory single-parameter arrows as source signatures; mint callable origins only from the selected configured source inventory.
4. Store portable source recipes (atoms, binding reads, member selections/calls and type tests), rebuilt through the existing source-content hydration path.
5. Infer ordinary arguments before deferred callbacks; substitute the candidate's current generic environment into contextual parameters.
6. Evaluate each callback with a private BindingId-to-TypeId overlay; do not mutate shared contextual facts while testing candidates.
7. Infer source return/predicate evidence, then check the existing full callable relation and generic constraints without erasing source origins.
8. Preserve incomplete bodies, unsupported control flow, annotations, access and overload ambiguity as explicit barriers.
9. Add failing source-origin/result/cascade tests, candidate-isolation negatives and portable/cold/poisoned-source checks; retain independent compiler labels.
10. Remeasure the fixed real-project cohort; callback-body support alone does not establish language overload ordering or the multilingual 99% gate.

```ts
api.pick(value => true);
// ❌ -> target: ✅ [generic] body evidence disproves a predicate competitor.
api.pick(value => typeof value === 'string');
// ⚠️ [generic + profile data] source predicate inference also requires independently
// attested overload ordering; never pick the first physical navigation row.
```

## Traced nullable-call continuation

1. The retained trace now proves Array.find yields QueryObserver | undefined; the remaining refetch misses are a nullable-receiver boundary.
2. Extend the configured bound-method adapter, not declaration-name fallback or global union erasure.
3. Use the existing source-extracted optional_chaining flag on the selected call segment.
4. Require accepted type context before projecting nullable union/optional operands.
5. Remove only typed Null/Undefined alternatives for optional-call member selection.
6. Preserve every non-null alternative; distinct receivers and unknown evidence remain barriers.
7. Reuse the existing owner/member-ID method and overload selection paths.
8. Restore the undefined branch in the call's yielded type when short-circuiting is possible.
9. Do not claim general optional-chain continuation, optional callable properties or control-flow narrowing.
10. Add a failing nullable-return cascade regression, then remeasure both retained refetch sites and the full fixed cohort.

## Implementation and limits

Binding epoch 61 / extraction schema 76. `lexical_callback_bodies` captures expression and single-return block bodies, atoms, exact binding reads, named property/method selections and primitive type tests. Single-parameter arrows now enter the source signature inventory. The existing content-backed portable hydration path rebuilds callback recipes; runtime TypeIds and BindingIds are never serialized as portable identities.

`program_callback_bodies` evaluates a callback under a private BindingId map. It preserves the callback's configured source/signature origin, source parameter slots and explicit annotations. Supported primitive-union type tests produce predicate evidence; they do not grant overload ordering. Declared public class method bodies are not interpreted: their source-owned signatures supply call results, with private/protected/static/optional and ambiguous selections fenced. Interface methods retain row-independent signatures.

The shared applicability path infers ordinary arguments before deferred callbacks, checks known mismatches, infers from callable results/predicates, and re-evaluates callbacks under the final substituted context before full assignment proof. The failing generic callback probe exposed both literal-candidate widening and stale contextual parameter types. Candidate evaluation never writes shared contextual facts; existing lambda publication runs only after unique overload selection.

The bounded optional-call adapter removes only Null/Undefined branches for member selection and preserves short-circuiting in the yielded type. Multiple present receivers, unknown/stale contexts and incomplete evidence remain barriers. This does not implement general optional-chain continuation or field/callable-property semantics.

Still unsupported: multi-statement/control-flow bodies, assignments, nested closures/calls without source method evidence, async/generator callbacks, generic callback instantiation, default/destructured/rest parameters, richer predicate inference, complete callable properties/access rules, general variadics and language overload ordering. The original inferred-predicate overload fixture remains a legal abstention; its compiler label is unchanged.

## Retained intermediate evidence

Before the optional-call adapter, the fixed configured cohort improved from 571 to 573 correct / 40 declaration-kind-only disagreements / 77 unresolved. Both recovered calls were the bodies of the onFocus/onOnline observer callbacks, with no other per-site changes and exact fresh/cold agreement. The next trace proved both local observer values now carry `QueryObserver<any, any, any, any, any> | undefined`; the remaining failure was optional-call receiver projection, not another missing callback type.

- `2026-09-08-query-core-callback-bodies-program-report.json`: SHA-256 `799a24379e270983142b8b01a4f26c53d77447273dd461fa46f177ae780bc86c`, 52,462 ms.
- `2026-09-08-query-core-callback-bodies-legacy-report.json`: SHA-256 `33ffd9810481b95ae6b5e7d6ca829c21450ad6b6ae83cf26361e4e226e7e22a9`, 52,219 ms; entire report unchanged from epoch 60 except elapsed time.
- `2026-09-08-query-core-callback-bodies-trace.json`: SHA-256 `5820ed729031b2f76bc48296bdc2cb5619deb0c948674e9cabec843f0719f56a`; 77 requested sites, 268 records, zero baseline or snapshot changes.

These timings are diagnostic runs, not controlled performance benchmarks. All intermediate reports are retained, not overwritten.

## Verification

Final selected library run: 2,109 passed, zero failures, 31 ignored, 34.39 seconds. All seven targeted integrations passed (`resolution_corpus`, `resolution_corpus_js`, `resolution_corpus_rust`, `per_file_manifest`, `per_package_context`). No workspace-wide, formatting or lint commands were run.

Pinned TypeScript 5.9.3 overload verifier: 30 cases, 26 exact selected-signature/result labels, four diagnostic-backed negatives, five legal engine abstentions. The original ordinary-arrow fixture is now supported without changing its target/result label. Seven new cases cover expression/block returns, candidate-local parameter reads, ordinary-argument-first inference, interface/public-class member calls and private-member rejection. The inferred-predicate fixture still abstains at competing overloads.

Source/body tests verify parameter-slot predicate identity, private candidate isolation across incompatible contexts, portable source reconstruction with shifted TypeIds and cold signature restoration. Semantic-model cascade tests cover declared and inline/block callbacks, local and inline result chains, nullable optional calls, exact downstream declaration IDs, poisoned display operands and fresh/cold snapshots. Source capture rejects mutation, async and nested unsupported returns; optional receiver tests reject unknown and multiple non-null alternatives.

Both intended regressions failed before their fixes: ordinary inline callback overload selection, then the nullable-return downstream call. The generic callback fixture also failed until candidate widening and final-context re-evaluation were both implemented. Native whole-worktree audits checked 330 changed production files and 125 resolver/type-system files: zero file-budget violations and zero increased string-identity patterns. `git diff --check` passed.

No new public integration API was introduced. The pre-existing frozen AlphaT/Lynx dependency/lockfile blockers remain unresolved; this turn's engine tests are not evidence of downstream application compilation.

## Final configured cohort

`2026-09-08-query-core-callback-bodies-verified-program-report.json`: SHA-256 `d2e2b4994344dc2fc0473e31e15924b497d304adb238462b608bd2f95cde8dfa`, 50,283 ms. Correct bindings increased from 571 to 575 of 690 (83.3333333333% recall, 93.4959349593% strict precision), with 40 unchanged declaration-kind-only disagreements and 75 unresolved. Exact per-occurrence comparison proves all four changes are unresolved-to-correct, with no other labelled changes and identical fresh/cold arrays.

The four recovered sites are in `packages/query-core/src/query.ts`: callback calls at bytes 7893 and 8119, and their downstream optional `refetch` calls at bytes 7936 and 8160. This closes the retained onFocus/onOnline callback-return cascades. Array.find overload selection also now supplies the correctly nullable local receiver; the existing navigation oracle does not independently score every overload signature, so selected-signature fixtures remain a separate gate.

The 99% gate remains false. Next is source-attested declaration-group ordering and language specificity/subtype rules for competing applicable overloads, beginning with the unchanged inferred-predicate arrow fixture. Broader body/control-flow inference, higher-rank callbacks, general optional-chain continuation, callable properties, variadics, Promise cascades, multilingual validation, IDE APIs, benchmarks and cross-service flows remain required.

Final legacy verification: `2026-09-08-query-core-callback-bodies-verified-legacy-report.json`, SHA-256 `f552ab939c48745df9a36bd3f2e53937dc31cbb93ef855dfcc18e5dfbf3d99f9`, 55,391 ms. The entire report equals the epoch-60 legacy report except elapsed time: 557 correct / 40 kind-only disagreements / 93 unresolved, with exact fresh/cold agreement. The fixed compiler manifest remains SHA-256 `44158824a3f91fdeda2d5e5da8a422054e178c76de29857c2f855ffe36a5110c`; all configured expected labels are unchanged. Final process audit found no remaining cargo/rustc/project-oracle/project-trace process.
