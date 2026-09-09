# Source-owned local value provenance

1. [generic] Replace the copy-only local initializer recipe with an ID-bound expression recipe carrying reads and reference construction.
2. [profile data] Recognize identifier, grouping and explicit borrow syntax through existing namespace/borrow forms; no language-name branches.
3. [generic] Capture every operand BindingId at its own source position, preserving shadowed initializer reads and nested reference layers.
4. [generic] Carry a borrow's source span, mutability and attested physical caller slot; unavailable owners remain unknown.
5. [generic] Lower caller slots to existing declaration IDs once per file environment, separately from runtime operand type evaluation.
6. [generic] Evaluate local recipes against position-correct binding facts without moving the inference cursor or resolving a display name.
7. [generic] Preserve annotations and authoritative unknowns, depth guards and existing assignment/function isolation; this is not a borrow checker.
8. [generic] Rebuild recipes from source through filtering/cold caches and invalidate old binding/extraction epochs.
9. [evidence] First independently compiler-label shared/mutable/nested/shadowed/imported/call-produced local cascades and negative controls, then assert exact type/region identities.
10. [boundary] Full place expressions, closure declaration ownership, pattern projections, control-flow joins and region/coercion/outlives proofs remain separate open work.

## Implemented contract — 2026-09-07

`TypeUses.values` replaces `value_copies`; `ValueExpr<Owner>` contains only
`Read { binding, byte }`, `Borrow { owner, span, mutability, operand }` and
`Unknown`. Ingestion owns physical declaration slots; the per-file lexical cache
lowers these once to existing declaration IDs before evaluating operand types.
Reads consult the source binding at its initializer position, not at the current
query cursor. Nested borrows keep separate source regions and reference layers.
No string lookup is used to evaluate these recipes.

The generic initializer capture reuses profile identifier/group/borrow forms.
Only a surviving, source-attested function owner can supply a borrow region.
Raw borrows, malformed or unsupported operands, missing owners and closure-owned
borrows without a physical closure identity remain unknown. Capture/lowering and
combined binding/expression evaluation have explicit depth limits.

An annotation remains authoritative about nominal identity and mutability. The
new `refine_regions` step fills unknown reference regions only where the declared
and initializer shapes correspond exactly. Different nominal declaration IDs,
mutability or known regions cannot be reconciled by spelling; unknown pointees
cannot establish equality. This is source provenance, not outlives/coercion proof.

```rust
let r = &p; r.touch();          // ✅ [generic] borrow recipe → operand BindingId → exact reference TypeId
let n = &r; identity(n).touch();// ✅ [generic] nested regions survive copies and generic returns
let p = &p; identity(p).touch();// ✅ [generic] initializer reads the preceding p binding, not itself
let r: &a::Doc = &p;            // ✅ [generic] retain a::Doc's ID and fill its unknown reference region
let raw = &raw const p;         // ⚠️ [profile data] excluded from reference construction; no member leakage
let closure = || { let r=&p; }; // ⚠️ [generic] closure declaration ownership remains open
```

Binding epoch 25 and extraction schema 45 invalidate earlier cached inputs.
Contract filtering and portable cache hydration recapture source expressions:
surviving caller slots are retained, removed nested owners become unknown, and
copies still reference the correct source binding and byte position.

## Evidence

- Nine pinned-rustc-labelled cases add 22 positive targets and one diagnostic
  negative. Before: 9/22 correct, 13 unresolved, zero wrong; after: 22/22 correct,
  zero wrong, one correct negative, fresh/cold equal.
- The first implementation closed all but the annotated-local miss. An exact-ID
  test proved that its nominal ID was correct and only its region was unknown;
  constrained annotation refinement closes that cascade without guessing a type.
- Unit tests cover source owner slots, shadowed reads, mutable/nested references,
  unknown/legacy operands, removed owners, cursor non-interference, prior versus
  later facts, cycles and depth limits, and annotation mismatch barriers.
- Renamed-import and namespace local-return cascades follow actual provider
  signature edits from module a to b, filtered providers, deletion and fresh-arena
  cold reload. Both direct-argument and borrowed-local variants remain covered.
- All 33 argument/qualified/borrow/local-value cases retain identical oracle
  reports after stored annotation text and reference/segment display arguments
  are poisoned (or arguments cleared), fresh and cold.
- Targeted core/profile verification: 2,154 passed, 25 ignored. Independent rustc
  1.94.0 verification: 78 cases / 179 positive targets / 15 negatives; all 12
  compiler-adapter tests pass. Native source-budget/ID audit: 249 production files,
  zero violations; whitespace diff check passes.

The unchanged earlier 69-case cohort remains 155/157 correct. Combined diagnostic
evidence is 177/179 correct positives, zero wrong, 15 correct negatives and 194
extracted sites. The only retained misses are the two match-pattern payload sites.
No representative per-language/global 99% claim follows from this authored cohort.

Remaining work: field/dereference/place operands, closure identities, pattern
projections, assignment/CFG semantics, composite local annotations and complete
region/coercion/outlives obligations, alongside the broader F0–F3 backlog.
