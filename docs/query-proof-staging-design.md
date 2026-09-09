# Proof-aware private value-query staging

1. Generic engine work only: configured views, merge proofs and source-bound query readers; no profile additions or library-name hooks.
2. Keep source/program generic owners allocated once during a private build; rematerialize dependent recipes without inventing new identities each round.
3. Keep physical declaration rows separate until final admission; a repeated property supplies a private query value only after complete compatibility proof of its named-property group. Whole-owner rejection still revokes all dependent facts.
4. Preserve rowless source/signature origins: query evidence is keyed by receiver declaration ID and MemberNameId, not a selected navigation row.
5. Feed successful receiver-specific inherited property projections into the same private query evidence, including substituted generic values and optionality.
6. Rebind all source value queries and computed keys against each evidence round; refresh signatures, constraints, bases and nominal inventories when results change.
7. Delay candidate rejection until query/proof dependencies stabilize; after rejection discard all evidence and rebuild, so rejected providers cannot sustain dependents.
8. Keep duplicate, access, callable and unknown barriers authoritative; no first-candidate selection, string recovery or provisional publication.
9. Bound staging and recursion explicitly; failure to stabilize is unavailable evidence, never proof of equality or a publishable partial snapshot.
10. Verify compiler-labelled repeated-property cascades, incompatible/cyclic providers, inherited refinements, edits/deletion, portable rowless origins and fresh/cold parity; then remeasure the unchanged real corpus.

```ts
declare const token: unique symbol;
interface Keys { tag: typeof token }
interface Keys { tag: typeof token }
declare const keys: Keys;
type Tag = typeof keys.tag;
interface Uses { [keys.tag](): void }
// ✅ [generic] named-property compatibility proof → receiver/member IDs → Tag and computed key.
// ⚠️ [generic] incompatible or cyclic evidence must never become an arbitrary first declaration.
```

## Implemented mechanism

`program_merge_proof::settle` alternates property/heritage proofs and source query binding inside an unpublished view. Generic owners and the member-name arena stay fixed across rounds. Query changes trigger replacement of materialized signatures, constraints, bases and nominal inventories. `MemberIndex::restore_projection` restores declared rows without losing appended source-name IDs. The singleton fallback in `program_value_queries::Solver::definition` and duplicate-signature fallback in `computed_keys::Values::member` remain guarded; only a separately proved receiver/member value bypasses that fallback.

Named-property groups can be proved before an unrelated computed key in the same interface is known. That permits productive self-dependencies without treating a recursive unknown as equality. All source interface parts participate, including module-local interfaces outside the global candidate list. Whole-owner compatibility and heritage checks still run after stabilization; rejection clears the private evidence by rebuilding from source. Physical repeated-property rows are canonicalized only after final admission; rowless recipes do not acquire invented navigation rows.

```ts
declare const token: unique symbol;
interface Keys { tag: typeof token }
interface Keys { tag: typeof token; [keys.tag](): void }
// ✅ [generic] local property proof seeds a productive dependency; final owner proof still required.
interface Cyclic { tag: typeof cyclic.tag }
interface Cyclic { tag: typeof cyclic.tag }
declare const cyclic: Cyclic;
// ✅ [generic] no independently proved value exists; recursive queries remain unresolved.

class Subscribable<TListener extends Function> {
  protected listeners = new Set<TListener>();
  // ❌ [generic] configured fields still need a source-owned constructor-initializer recipe.
}
```

Staging is bounded at 128 rounds. Exhaustion returns an unavailable view rather than publishing partial facts. A retry test verifies exhaustion is not a semantic success and does not reallocate generic owners. Complete expression inference, callable values, new structural origins, general language semantics and performance optimization of large dependency graphs remain separate work.

## Verification

The original repeated-property alias/value/key cascade failed before implementation. A second compiler-legal productive self-dependency failed until property proof was separated from whole-owner admission. An imported, module-local repeated-property test exposed that global candidates alone were insufficient; it passes with the shared source-interface inventory and full rejection checks.

Pinned TypeScript 5.9.3 independently verifies ten cases: five legal sources with six exact unique-symbol computed-key origins, and five diagnostic-backed negatives. Tests cover multi-round dependencies, inherited required/generic refinements, optional/incompatible/cyclic barriers, productive cycles revoked by an unrelated invalid member, and module-local admission/rejection. Source tests additionally cover unchanged-consumer provider edits, rollback by deletion, barrel retargeting, deleted namesakes, overlapping programs, filtered rowless signatures, portable arena remapping, poisoned display metadata and cold reload. The pre-existing 25 merge, 26 heritage, 41 assignment and 11 obligation compiler controls pass unchanged.

Final selected library verification: 2,074 passed, zero failed, 31 ignored, 5,623 filtered; 31.47 seconds. Seven selected integrations pass. Native audits found zero file-budget violations across 317 changed production files and zero added string-identity patterns across 118 scoped production files; `git diff --check` passed. Binding epoch is 55; extraction schema remains 71. The earlier AlphaT/Lynx consumer compilation caveat remains unresolved; no sibling lockfiles were changed.

## Real-project measurements

Both complete reports are identical to their preceding mixed-heritage reports after removing only `elapsed_ms`; this includes every original label and all fresh/cold occurrence records, not just counts. There are no new source-input gaps or snapshot differences. The two previously corrected QueryObserverOptions sites remain corrected. These are development diagnostics, not performance benchmarks or representative 99% evidence.

- Configured: `resolution-documents/2026-09-08-query-core-query-proof-program-report.json`; 548 correct, 40 declaration-kind-only disagreements, 102 unresolved out of 690 labels. Recall 79.4202898551%, strict precision 93.1972789116%; gate false. SHA-256 `1657b2527f000f5d7a4fab31e6c0b4e8f90a5921dcfd451f60fd2c42f5bcc7e6`; elapsed 41,748 ms.
- Legacy: `resolution-documents/2026-09-08-query-core-query-proof-legacy-report.json`; 557 correct, 40 kind-only disagreements, 93 unresolved. Gate false. SHA-256 `013f0f314ec149f4285220b107f7a46ca45e3c638bc4f6fbd2ee2c69a50d1681`; elapsed 58,846 ms (overlapped integration verification).
- Original compiler manifest unchanged: SHA-256 `44158824a3f91fdeda2d5e5da8a422054e178c76de29857c2f855ffe36a5110c`.

## Next root cause

The actual pinned `subscribable.ts` has `protected listeners = new Set<TListener>()`. `lexical_type_syntax::TypeUses` captures exact call-initializer addresses, but configured `program_types::Input` materializes annotated field/signature recipes rather than constructor initializer yields. Legacy initializer inference writes workspace `TypeInfo`; it must not be borrowed into the selected program. Next work must carry the runtime value head and full construct-signature evidence into source-owned initializer recipes, preserve explicit generic IDs and annotations, and evaluate the resulting `Set<TListener>` field before dependent member chains. Merely resolving a type named Set would not prove the constructor's result.
