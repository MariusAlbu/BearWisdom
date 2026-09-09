# Source value-domain binding

1. Trace private merge failures before changing acceptance: retained Set/Map/Promise failures share unresolved Symbol-valued computed keys.
2. The source captures those roots as the same-file interface BindingId because reference_binding_at falls back to type entries.
3. Add a generic source value-expression lookup that traverses value entries and import-shadow barriers by ScopeId/NameId.
4. Pure type declarations do not hide a value supplied by the selected program; missing local values become explicit Global(NameId) recipes.
5. Keep type-only imports authoritative rather than searching past them for an outer/global namesake.
6. Reuse the lookup for computed keys and source typeof paths without changing legacy reference APIs or other language policies implicitly.
7. Preserve value parameters, classes, ordinary imports, declaration ordering, capture barriers and separate type-name lookup.
8. Version the changed source-binding recipes and cover fresh/cold, filtered/portable, edited/deleted and overlapping program inputs.
9. Pin independent compiler targets, type/value shadowing controls and diagnostic-backed import negatives; no builtin API list or runtime spelling recovery.
10. Remeasure every retained occurrence and trace remaining initializer/heritage/merge barriers; full F0–F3 goals remain active.

Compiler correction and implementation contract:
- TypeScript 5.9.3 accepts type-only imports in interface/type-literal computed keys, abstract/ambient members and typeof queries. The initial expected diagnostic 1361 for an interface key was wrong; retain it as a positive and add actual runtime-key negatives.
- Add a separate ValueQuery export domain: it follows the value facet of the selected imported entity through type-only imports/exports, never the entity's type facet and never an outer namesake. Runtime Value bindings keep their existing prohibitions.
- Carry a source-captured Runtime/Erased use kind on computed paths. Generic capture reads profile-declared erased containers/modifiers and ambient syntax; typeof is explicitly Erased. No runtime display-text checks.
- Extend module input lowering, scoped forwarding, wildcard and export-assignment edges together. Keep ambiguity, cycles and incomplete providers authoritative in the additional domain.
- Namespace expressions in erased computed keys still select runtime-exported members; only typeof query paths traverse value-query export members. Compiler diagnostic 2339 protects this distinction.
- Repair the pinned TS/TSX grammar for type-only wildcard/namespace exports, preserving export_statement/source fields and requiring a from clause. Regenerate from the pinned local toolchain; do not suppress parse completeness errors.
- Verify compiler-labelled import, re-export, namespace and type/value collisions; fresh/cold, poisoned metadata, portable cache, provider retargeting/deletion and program isolation remain required.
- An explicit type-only export installs an authoritative missing runtime export rather than being omitted. The compiler-backed `export type { tag } from left; export * from right` runtime key control exposed a wrong-owner fallback; the value-query domain still selects left.

## Implemented and verified mechanics (2026-09-08)

`LexicalBindings::value_expression_binding_at` walks scope/name IDs, skips pure type declarations and stops at nearer import bindings. The old reference API is unchanged. Source computed paths retain Runtime/Erased/Query use kinds; profile data supplies type-literal/interface containers, abstract/declare modifiers and ambient declaration syntax. Module inputs, scoped forwarding, assignment entities, wildcard exports and ambient provider groups carry the separate ValueQuery domain. This selects value entities without turning type-only imports into runtime values or borrowing their type-space namesakes.

```ts
import type { Token } from './provider';
interface Uses { [Token.tag](): void; }
// ✅ [generic] Source Erased use selects imported value-entity IDs.
// [profile data] The interface container supplies the erased-use evidence.

class Runtime { [Token.tag](): void {} }
// ✅ [generic] Runtime Value domain remains unresolved; compiler diagnostic 1361.

import type * as ns from './type-only-barrel';
declare const alias: typeof ns.tag;
// ✅ [generic] Query domain follows type-only exports through namespace IDs.
interface Invalid { [ns.tag](): void; }
// ✅ [generic] Erased key namespace selectors still require runtime exports;
// compiler diagnostic 2339 prevents a guessed member.
```

- The corrected fixture first failed with `Some(None)` instead of its independently verified unique-symbol TypeId.
- Type-only wildcard syntax first failed complete-source capture. The grammar now preserves export_statement/source anchors in both dialects; nine grammar tests pass, including malformed and incremental controls. Regeneration reproduced 17 generated/query files byte-for-byte.
- The wildcard namesake negative first returned the right-hand provider's unique TypeId instead of unresolved. Explicit missing runtime exports now block that path.
- Final selected library suite: 2,027 passed, zero failed, 30 ignored. Fourteen new compiler-validated value-domain cases contain eleven exact unique targets, one exact intrinsic query result and four diagnostic-backed negative keys. Existing nine value-query cases and seven rich-merge cases / nineteen exact calls also verify independently with TypeScript 5.9.3.
- Seven integration tests pass across resolution_corpus, resolution_corpus_js, resolution_corpus_rust, per_file_manifest and per_package_context. Native audits found no file-budget violations across 305 changed production sources and no increased string-identity patterns across 109 resolver/type-checker sources; whitespace checks pass. Existing sibling-consumer lockfile compatibility remains outside this verification, as recorded in the roadmap.
- New state tests cover unchanged-consumer type-only export retargeting, deletion with a live global namesake, runtime rejection, portable/cold program value identities, overlapping program fences, and ambiguous/cyclic type-only namespace providers. Existing shared tests retain poisoned-display and fresh/cold checks.
- Binding epoch 48 / extraction schema 68 invalidate old source capture. Computed-key queries in unavailable program views now return authoritative unresolved results consistently with source typeof queries.
- The pinned real-library private-view diagnostic now accepts Map, Set, Promise, PromiseConstructor and SymbolConstructor in both rejection rounds. This proves the traced admission barrier closed; occurrence gains require the separate unchanged-manifest report.

## Remaining scope

This does not migrate all legacy value-expression APIs or establish general type-only/JSDoc/non-emitting-heritage semantics. Implicit declaration-file ambient context without explicit source modifiers needs separate configuration evidence. Full callable/structural values, inferred unique initializers, legal heritage/generic-rich merges, repeated-property query dependencies and canonical merged unique origins remain open, along with initializer-derived fields and the full F0–F3 roadmap.

## Unchanged-manifest real-project measurement

Manifest `resolution-documents/2026-09-07-query-core-configured-compiler-manifest.json` remains SHA256 `44158824a3f91fdeda2d5e5da8a422054e178c76de29857c2f855ffe36a5110c`: 187 supplied sources, 24 selected files, 843 compiler calls, 690 labels and 153 unlabelled calls. Each run verifies source hashes before and after evaluation.

- Configured report `2026-09-08-query-core-value-domain-program-report.json`, SHA256 `a1249847c9828b30c87089160110d38ffae35789b3fef32fd70d897c8ba351ab`: 492 correct, 40 unchanged declaration-kind disagreements, 158 unresolved; 71.3043478261% strict recall / 92.4812030075% precision. All 46 changed labelled references are unresolved-to-correct in both fresh and cold snapshots; every expected label remains identical. All other non-timing/non-occurrence report metadata deep-equals the preceding merge-property report. No configured source gaps or snapshot changes; gate remains false.
- The recovered targets comprise 26 Promise calls (reject fourteen, then seven, catch five), fifteen Map/Set member calls, two Array.isArray calls and three Math min/max calls. This is one shared source binding/merge cascade, not hand-maintained API special cases.
- Legacy report `2026-09-08-query-core-value-domain-legacy-report.json`, SHA256 `4ac4ba3c993a049e9c968b10d25ad5bfc59cb98b6b33f81baa617e82883d4da9`: 557 correct, 40 kind-only disagreements, 93 unresolved, zero snapshot changes. The entire report deep-equals the preceding legacy report except elapsed time.
- Elapsed times were 39,891 ms configured and 53,970 ms legacy. These are uncontrolled development runs, not comparative performance or token-efficiency evidence.

The next retained source barrier is the Node compatibility declaration `interface Array<T> extends RelativeIndexable<T> {}` (and ReadonlyArray). Interface heritage needs type-domain source heads, generic applications and inherited compatibility proof, not removal of the plain-header guard. `Subscribable.listeners` is initialized by `new Set<TListener>()` and still needs a source-owned initializer-derived field recipe; admitting Set alone does not supply that field type.
