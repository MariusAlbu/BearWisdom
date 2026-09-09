# Enclosing declaration identity across snapshots

1. [generic] Reproduce fresh/cold class-member cascades through the real-project oracle runner before changing resolution.
2. [generic] Restore nearest enclosing type IDs from physical containment/member ID edges, never qualified-name lookup.
3. [generic] Reuse Compilation's complete symbol and containment indexes after DB rows have been loaded.
4. [identity] Keep freshly parsed declarations authoritative; restore only DB-loaded source declarations.
5. [identity] Walk through non-type ancestors and choose the nearest type declaration, preserving same-spelling owners.
6. [safety] Missing, conflicting and cyclic containment evidence cannot supply a guessed enclosing type.
7. [integration] Restore ancestry before canonical merge and detached-owner rebinding so existing identity passes remain authoritative.
8. [tests] Cover forward parents, nested declarations, namesakes, malformed ancestry and mixed fresh/DB snapshots in sibling tests.
9. [evidence] Rerun the pinned Query Core manifest to a new report and compare every labelled site without changing gold labels.
10. [boundary] Superclass selection, inherited generic arguments, CFG and full configuration parity remain separate evidence-driven work.

## Cause and implementation

Fresh `Compilation::ingest` populated `enclosing_type_by_id` from `ParsedFile.parent_index`. `ingest_from_db` restored `members_by_id` from `symbols.containing_id` but did not derive enclosing type IDs. Simple examples could still resolve via legacy scope/name fallback, hiding the missing identity relation.

`enclosing::restore_enclosing_types` now inverts numeric member/containment edges and walks to the nearest live type declaration. It memoizes results, rejects conflicting parent IDs and terminates cycles or missing ancestry. It restores only DB-loaded sources; both present and absent fresh facts remain authoritative. The pass runs before canonical merging and detached extension-owner rebinding. No new schema, source extraction format, language hook or name-based identity relation is introduced.

```typescript
this.setOptions(options);    // ✅ [generic] enclosing_type_id_of → declaration ID, fresh and cold
this.cache.notify(event);    // ✅ [generic] restored root ID → field TypeId → member ID cascade
super.destroy();             // ❌ [generic/profile data] TS extraction currently tags super as SelfRef;
                             // chain_root_binding roots on the child, not the parent (separate work)
```

## Evidence

- First direct ancestry regression failed: cold `a.ts` method had `None` instead of its enclosing class ID. Two same-spelling classes in separate files stay distinct in fully cold and mixed fresh/DB snapshots after the fix.
- Sibling tests cover intermediate function ancestors, nearest nested classes, conflicting/missing parents, self/two-node cycles, parent removal and preservation of fresh facts, including deliberately absent fresh entries.
- A three-call source-level field/return cascade passes the production oracle runner fresh/cold. This simple fixture passed before the fix through fallback and is not presented as the original failing reproduction.
- The frozen Query Core real-project manifest provides the cascade evidence: 314 snapshot differences before, zero after. Cold correct labels improve 243 → 552; all fresh per-occurrence observations remain byte-for-byte JSON-equivalent. Five previously wrong targets are also restored, not fixed.
- Strict after-fix counts: 552 correct, 45 mismatches, 93 unresolved out of 690 labels in both snapshots. Forty mismatches are declaration-kind-only; all five different source locations are superclass-method calls selecting child overrides.
- Selected core/profile/oracle/query suite: 2,191 passed, 25 ignored. Real-project compiler adapter tests: five passed, including inherited tsconfig inputs, renamed imports, support-only declarations, BOM and Unicode byte coordinates.
- Twelve integration tests passed across incremental indexing, per-file/per-package contexts and TS/JS/Rust resolution corpora. Native file-budget/ID audits passed for 259 changed production files; `git diff --check` passed.
- Downstream locked/offline checks stop before compilation: AlphaT cannot select yanked `der 0.8.0` via `ureq`/`ort`; Lynx requires a lockfile update. Neither lockfile was changed, and downstream compatibility is not claimed verified.

## Remaining scope

This closes one demonstrated snapshot-loss cause, not the F2 dependency/snapshot milestone. Persisted inherited generic arguments, full configuration fingerprints, edits/deletions across semantic dependency closures, all-language identity coverage, superclass dispatch and elimination of remaining legacy root fallbacks still need their own evidence and tests.
