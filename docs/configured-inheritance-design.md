# Configured inheritance from source-owned recipes

1. [evidence] Actual FocusManager captures its imported base but configured TypeInfo drops it; inherited calls and their generic yields therefore fail upstream.
2. [generic] Capture configured base inputs alongside program signatures, sharing declaration/value-import head lowering with the existing base receiver path.
3. [identity] Lower generic arguments to program Recipe owner/index/global/import identities, never persist workspace Fixed TypeIds as configured arguments.
4. [generic] Materialize direct base applications only after program source environments and generic owners exist.
5. [identity] Store parent declaration IDs and argument TypeIds in the configured view; expose existing numeric parent APIs without string-query fallbacks.
6. [generic] Reuse inherited_bindings substitution for configured method selection so the declaring ancestor receives the right generic environment.
7. [safety] Preserve unknown, missing, wrong-kind, conflicting and cyclic evidence; never revive rejected provider or source identities.
8. [snapshot] Version recipes, verify source edits/deletion, overlapping program selection, poisoned display data and fresh-arena cold/portable reload.
9. [oracle] Run independently compiler-validated base receiver cases through configured evaluation, plus direct inherited-instance and generic callback cases.
10. [gate] Recapture the unchanged real configured and legacy populations; distinguish inheritance gains from remaining stdlib merge and initializer failures.

## Inherited field selector follow-through

The expanded poisoned-display gate fails on `this.item.touch()` while all three
`super.map(...)` callback/return sites remain correct. `chain.rs` selects called
members from source selector IDs, but non-call hops still pass `seg.name` to
`implicit_root::walk_member`. The already-captured source selector NameId is
available for `item`; no new parser rule or string recovery is needed.
Expose that selector-to-MemberNameId mapping through the internal flow contract,
and let the bound-member adapter select the field by ID before the legacy walk.
Captured missing/ambiguous/inaccessible selectors must terminate that path.
Keep the shared inherited substitution for the field yield; gate both own and
inherited field cascades, fresh/cold and with poisoned display text.

## Implemented and verified, 2026-09-08

- `program_types::bases` captures declaration/value-import heads and source-owned
  generic argument recipes. `program_view` materializes them in the selected
  program, then installs parent declaration IDs and argument TypeIds.
- `program_lookup` exposes numeric parents/arguments to member selection and
  inherited generic substitution. Missing/non-class/self/conflicting/cyclic
  bases do not create parent edges. No profile or language hook was added.
- The internal source-member selector contract removes display-name lookup from
  configured non-call hops. A missing captured selector is authoritative even
  when its unchanged display spelling would find a member.
- Binding epoch 37 and external extraction schema 57 invalidate older inputs.
- TypeScript 5.9.3 independently verifies the original 13 inheritance cases / 25
  labels and four new cases / 11 labels. All 36 labels match fresh and cold;
  poisoning qualified names, signatures and selector display text changes none.
- Parsed provider late arrival, barrel retargeting, provider deletion and
  unchanged caller recipes are covered through exact downstream call targets.
  Overlapping programs select distinct parents and program-owned argument
  TypeIds for one physical source. Filtered portable providers retain ancestor
  generic-owner IDs through a fresh arena and a cold database snapshot.
- Selected core/profile/oracle run: 1,949 passed, 28 ignored; seven integration
  tests passed. Source budget / ID-pattern audit: 287 / 98 files, no violations.

The real-provider diagnostic now finds a configured base application for
FocusManager. `Subscribable.listeners` still has no configured field type, and
Array/ReadonlyArray/Set/Map/Promise/PromiseConstructor type groups remain
incomplete. Captured inheritance is not permission to accept those groups.

Remaining inheritance scope includes cross-file global value heads that lack
a lexical BindingId, interface/multiple bases, mixin expressions, complete
generic defaults/constraints and language-specific legality/precedence. The
authored cohort is diagnostic evidence, not representative 99% coverage.

## Real-project regression and private identity follow-through

The first real recapture found 26 corrected calls but 97 regressions (354 to
283 correct). Its report and trace are retained as failure evidence. The trace
roots `this` correctly, then declines at `#mutationCache`; the selector profile
only captures public identifiers. The new ID-only field path exposed that gap.
Private names cannot simply join public member names: Parent.#item and
Child.#item are different lexical declarations even on the same Child receiver.
Capture private selectors against enclosing class-body declarations by source
coordinates, lower surviving slots to declaration IDs, and validate the receiver
contains that exact declared member through numeric parent edges. Missing lexical
owners, duplicates and unsupported static private declarations remain unknown.
Use profile token/class data, no TypeScript branch in the generic algorithm.
Protect private brands and ordinary field cascades with independent compiler
targets, then repeat the unchanged real population without discarding failures.

## Private selector closeout

`lexical_private_members` attests the exact enclosing class-body declaration by
source position. The generic ingestion pass captures either its source slot or
an authoritative miss; the TypeScript/JavaScript profile supplies private-token
and static-modifier kinds. FileLookup lowers surviving slots to row IDs, and
configured private selection validates exact containment through numeric bases.
Ordinary MemberNameIds cannot substitute a child's different private declaration.
Static private declarations and duplicate accessor/declaration shapes abstain.

The compiler cohort now has 19 cases / 43 labels, including private field return
cascades and parent/child private brands. All match fresh/cold and poisoned
display variants. Scope/brand rejection and missing-selector negative tests pass.
Final epoch 38 / schema 58 verification: 1,951 selected tests, 28 ignored; seven
integrations passed; 288 source / 98 ID-pattern files audited without violations.

The unchanged 187-source / 24-selected-file / 843-call / 690-label manifest yields
436 correct, 40 unchanged kind-only disagreements, 214 unresolved in configured
mode (63.1884% correct recall, 91.5966% strict binding precision). Every prior
correct occurrence is unchanged; 82 unresolved occurrences became correct. All
97 temporary regressions are repaired. Fresh and cold reports are identical;
source-input gaps remain zero. This is development evidence, not the 99% gate.

Final configured report:
`resolution-documents/2026-09-08-query-core-inheritance-private-id-program-report.json`
SHA256 `bc5432d84bc62f0fb6e609d2906d4c15ba62101d6ffcffeb4abbc81a2a49878c`.
The earlier `configured-inheritance-program-report.json` and corresponding trace
remain immutable failure evidence (283 correct, 367 unresolved). Next work must
address the remaining actual type/initializer causes rather than relax capture
completeness, restore display fallback or count kind disagreements as correct.

The final legacy report remains 557 correct / 40 kind-only / 93 unresolved,
deep-equal to the prior legacy report after excluding only elapsed time:
`resolution-documents/2026-09-08-query-core-inheritance-private-id-legacy-report.json`
SHA256 `48cac5d00ad2c3b77b96247b997abaeed8195b0f2e1e1ac7964f792f4ba4cb14`.
The final configured trace report covers all 214 unresolved oracle occurrences,
with 880 diagnostic records (including repeated phases), zero baseline changes
and zero snapshot changes. Its embedded evaluation equals the ordinary report
except elapsed time. Trace records are not independent occurrence counts.
Manifest SHA256 remains
`44158824a3f91fdeda2d5e5da8a422054e178c76de29857c2f855ffe36a5110c`.
