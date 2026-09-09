# Source-owned value queries and dependent signature binding

1. Capture type-query operands from profile syntax while lexical BindingIds and source spans exist; no semantic decoding of displayed types.
2. Reuse computed-key root/selector inputs for typeof, including scoped import bindings and namespace export selections.
3. Intern member/export names at view construction; traverse only source/module/binding/declaration/member/type IDs afterward.
4. Separate source value recipes from their materialized results, so queries can follow annotations across declaration order and files.
5. Evaluate source queries with memoized source-span keys, declaration/alias dependency keys and cycle/depth guards.
6. Materialize recipe children through a query callback, preserving nominal/generic owners and unknown operands without iterative guessing.
7. Reuse this evaluator for computed keys and named source member values; static constructor values remain distinct from instances.
8. Install all query results before final configured signature materialization; do not expose candidate merge groups or alter merge guards.
9. Verify forward/multi-file queries, renamed namespace imports, aliases, cycles, shadowing, filtered/portable/cold inputs and exact compiler targets.
10. Preserve the full correctness/product roadmap; complete callable/structural values and staged rich-merge compatibility before claiming general typeof or 99% coverage.

Syntax ownership correction: the pinned recursive type-query CST prints the right
outer property but `child_by_field_name` can return an inner inherited field.
Shared lexical selector capture now walks direct cursor fields for both detached
import bindings and value-query/computed-key paths. This is generic ingestion,
using existing profile field names, not expression-text reconstruction or a hook.

## Implemented source-query path

```ts
declare const first: typeof second; // ✅ [generic] source-span dependency, not declaration iteration order
declare const second: typeof token; // ✅ [generic] memoized query → annotated binding recipe
declare const token: unique symbol;
type Key = typeof provider.inner.tag; // ✅ [profile data + generic] direct CST fields → BindingId/export-name IDs
class Derived extends Holder<typeof token> {} // ✅ [generic] base arguments share the query evaluator
declare const opaque: unknown;
declare const copy: typeof opaque; // ✅ [generic] Intrinsic::Unknown, not unresolved Type::Unknown
type FunctionObject = typeof declaredFunction; // ⚠️ [generic] full declaration callable/overload values remain open
```

The selected program allocates source unique-symbol and generic identities, then
evaluates query/computed-key dependencies from raw source-owned recipes before
installing final signature and base results. Roots, namespace exports, nominal
members, aliases and generic substitutions use IDs. Static constructor values
remain distinct from instances. Member signatures need not have navigation rows.

`program_value_queries` memoizes source/query spans, canonical field declarations,
source signature IDs and alias TypeIds. Active cycles fail closed. A 128-node
dependency bound records dependency height with cached results, so warming the
cache cannot turn an over-budget chain into an accepted result. Depth-exhausted
ancestors are not cached as context-independent misses. Namespace/member walks
retain their existing independent bounds and ambiguity fences.

The old unconfigured `TypeExpr::ValueQuery` display payload remains an explicit
migration boundary. Configured lowering drops it; configured misses never read it.
This is not a claim that every legacy string bridge has been removed.

## Verification, 2026-09-08

- Forward-reference regression failed before the source recipe evaluator (Cargo
  19201), then passed. The retained namespace regression failed in Cargo 82621:
  its outer `tag` selector had incorrectly become a second `inner` NameId.
- Direct cursor-field capture fixed both the detached namespace selector and
  query path. The focused nine-test run passed (Cargo 70814), including TS/TSX
  ownership checks, query cycles and cache warmup-order checks.
- An added independently legal inherited query failed before base arguments used
  the query callback (Cargo 78685); the dependency evaluator now covers that path.
- Cargo 44076 exposed a test-only attempt to clone non-Clone ParsedFile; the test
  now parses an actual changed source to prove the stale-hash barrier.
- Cargo 42636: all 17 focused selector/computed-key/query tests passed.
- Cargo 50126: 2,012 selected library tests passed, 29 ignored, zero failures.
- Cargo 59492: all seven selected integrations passed (resolution corpora for
  TS/JS/Rust, per-file manifests and per-package context).
- TypeScript 5.9.3 independently verifies nine shared value-query fixtures: nine
  exact unique declaration targets, four exact intrinsic query/binding types, and
  one diagnostic-backed invalid computed key. Parenthesized interface keys retain
  diagnostic 1169; comments without parentheses preserve the valid key identity.
- The previous four computed-key fixtures/seven exact targets and 15 unique-owner
  fixtures/eight origins/seven diagnostic negatives still pass their verifiers.
- Fresh and fresh-arena cold controls cover cross-file provider retargeting and
  deletion without consumer recapture, filtered/portable rowless members, poisoned
  display payloads, same-byte separate programs, overlapping-program rejection,
  actual source-hash changes, forward reads, cycles and ambiguous namespace roots.
- Binding epoch 46; extraction schema 66. All 16 touched production files meet the
  source-file budget; `git -c core.safecrlf=false diff --check` passes.

## Remaining semantic work

Full callable/overload and structural namespace/object values, type-only import
query domains, initializer-inferred unique values, literal/computed selectors,
generic defaults/constraints and operator evaluation remain incomplete. The
member reader remains conservative for optional/private/protected/duplicate
members and unsupported bases. These are not grounds for guessing a namesake.

Rich merge admission still needs an isolated candidate-binding phase and a proof
over bound property/index/overload/generic signatures. Canonical unique identity
for legally merged properties cannot be inferred just from distinct source
origins. The early `program_graph::bind_group` rejection guard remains intact.
Next, complete that proof and initializer-derived field/value recipes, then
remeasure the retained Set/Promise/Array/Subscribable.listeners cascades.

Consumer compilation remains unverified because of the previously recorded
AlphaT/Lynx dependency lock incompatibilities. No sibling source or lockfile was
changed. The full F0–F3 correctness, IDE, competitive and flow goals remain open.

## Real-project measurement

The unchanged compiler manifest retains 187 supplied providers, 24 selected files,
843 calls and 690 independent labels (153 unlabelled compiler sites). Manifest
SHA-256: `44158824a3f91fdeda2d5e5da8a422054e178c76de29857c2f855ffe36a5110c`.

Configured report: `resolution-documents/2026-09-08-query-core-value-query-program-report.json`
(Cargo 51354), SHA-256
`349e72ab6f7eb37a32fa4b6e6ff3406375a927c005443e198118eba2904cae63`.
436 correct, 40 declaration-kind disagreements, 214 unresolved, zero snapshot
changes. The entire JSON is deep-equal to the unique-key report after excluding
only `elapsed_ms` (37,628 ms). Recall remains 63.1884%; precision 91.5966%; the
correctness gate remains false. Timing is uncontrolled, not benchmark evidence.

No production code changed after the final library/integration verification.

Legacy report: `resolution-documents/2026-09-08-query-core-value-query-legacy-report.json`
(standalone current executable, session 96727), SHA-256
`d8dcaab894f6b4eeec115ba13aa534203066a893e03dd99beec61adf2b8fb07d`.
557 correct, 40 declaration-kind disagreements, 93 unresolved, zero snapshot
changes. The entire JSON is deep-equal to the unique-key legacy report after
excluding only `elapsed_ms` (56,322 ms). Recall remains 80.7246%; precision
93.2998%; the correctness gate remains false. Neither timing establishes a
performance comparison. All Cargo and oracle process handles are terminal.
