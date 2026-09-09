# Source-owned nominal surfaces for structural queries

1. Extend configured-program source materialization, not legacy display lookup: nominal surface owners are declaration IDs, named keys become TypeIds at ingestion, and navigation origins retain source/signature/physical-row identity.
2. Store private nominal inventories on View after source signatures materialize; do not reconstruct declarations from symbol names during a structural query.
3. Separate complete key inventory from lazy member-value evaluation. `keyof` depends on the declared key set; indexed access evaluates the selected member; full structural assignment evaluates the required surface.
4. Keep duplicate declarations, overload groups, optional/readonly/index axes and unsupported member kinds explicit. A surface is not an invented empty object or a declaration-merging certificate.
5. Substitute nominal generic applications/defaults through GenericParamIds and preserve source constraint obligations. Cycles, foreign contexts and rejected providers remain authoritative barriers.
6. Traverse source-bound base applications in declared order with bounded recursion; inherit only source-attested keys/member origins and retain conflicting evidence for private compatibility proofs.
7. Make the generic structural evaluator consume numeric nominal inventories for keyof/indexed access and assignment, without changing nominal declaration identity into object identity.
8. Integrate evaluated mixed nominal/structural interface bases only after compatibility and navigation provenance are proved; do not discard a structural branch or relax the nominal guard as a shortcut.
9. Add shared compiler-backed nominal-query cases, invalid/ambiguous controls and fresh/cold/portable/poisoned/edit/deletion checks before measuring the retained QueryObserverOptions failures.
10. Keep mapped-distribution provenance, numeric/string key equivalence, conditional/infer evaluation, repeated-property query staging and initializer fields explicit until implemented and independently verified; this step does not redefine the full 99%/IDE/AI/flows goal.

## Implemented source inventories and queries

`program_nominal_surfaces.rs` materializes interface inventories after selected-source signatures are available. Named keys become literal TypeIds at this ingestion boundary; queries retain source/signature IDs, optional physical declaration rows and MemberNameIds. Generic applications/defaults/constraints and source base recipes are carried into `program_structural_nominal.rs`. Key queries do not evaluate unrelated member values; indexed access evaluates the selected property and preserves optional reads.

The twelve-case pinned TypeScript cohort covers generic/default/inherited/merged inventories, finite mapped projection, optional reads, namesake rejection and lazy unselected conditional values. It passes filtered portable-cache reconstruction into a shifted arena, configured fresh/cold snapshots and poisoned display metadata. A rowless-member test verifies the original source signature survives without a navigation row. A provider-edit test verifies values and physical/source origins follow an unchanged consumer's barrel retarget, then an empty provider and deletion, without borrowing the remaining namesake.

This is not yet general structural assignability or mixed-base admission. Nominal call/construct signatures are deliberately excluded from the key inventory; full callable/structural surfaces must retain that separate evidence before being used as an assignment or inheritance certificate. Numeric/literal/computed-key completeness, duplicate-member diagnostics, mapped distribution, effective inherited modifiers and receiver-specific member-value projections remain open.

The real Query Core trace exposed a further upstream barrier in QueryKey's conditional/infer constraint. Its source-owned implementation and evidence are recorded in `source-conditional-inference-design.md`; no Query Core API-name hook was introduced.
