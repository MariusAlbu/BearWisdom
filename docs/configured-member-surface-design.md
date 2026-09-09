# Configured rich-member identity and merge evidence

1. [generic] The active F1 blocker is upstream: `lexical_globals::member_surface` drops unnamed/computed members and `program_graph::bind_group` rejects unproven merged groups.
2. [generic] Preserve every direct source member, including unsupported forms, with a source span identity independent of whether extraction emitted a navigation row.
3. [profile data] Classify property/method/getter/setter/call/construct/index forms, key syntax and modifiers using profile data; introduce no language-name resolver branches or API lists.
4. [generic] Intern named keys at ingestion; computed paths carry a source BindingId or global NameId plus selector NameIds, never a name-based runtime retry.
5. [generic] Preserve parameters, generic parameters, result/type syntax and initializer spans as source evidence; syntax capture alone is not a type-equivalence or unique-symbol proof.
6. [generic] Lower source member slots to exact persisted declaration IDs, retaining absent/ambiguous rows rather than correlating by display names.
7. [generic] Persist an explicit optional surface payload in configured program inputs; missing old payloads remain distinguishable from known-empty surfaces.
8. [generic] Keep current merge guards authoritative until program-owned computed-key binding, overload groups and compatibility validation consume this evidence.
9. [evidence] Independently compare source member kinds/anchors/signature spans with pinned TypeScript compiler ASTs, including the supplied real standard-library declarations; test filtering, portable caches and cold snapshots.
10. [evidence] Retain the immutable 690-label configured baseline, negatives and poisoned-display checks; only mark the capture prerequisite complete, never the parent stdlib/initializer or F0-F3 milestones.

## Traced baseline

The retained `2026-09-08-query-core-inheritance-private-id-program-traces.json` and
`diagnose_pinned_program_global_and_signature_barriers` identify rejected Array,
ReadonlyArray, Set, Map, Promise and PromiseConstructor type groups. Separately,
`Subscribable.listeners` has no configured field type for `new Set<TListener>()`.
The old capture layer retains only member names: call/construct signatures lack
that field, index parameter names are incorrectly treated as property names,
computed keys fail the name-kind check, and repeated names cannot be
distinguished from incompatible properties. Neither lowering these facts nor
parsing a provider cleanly is evidence that those declarations legally merge.

Inspection also exposed an existing acceptance hole: `[first: string]` and
`[second: string]` were treated as distinct named members. Capture now keeps the
index-domain barrier instead. No rich-member acceptance guard is relaxed.

## Implemented capture boundary

`lexical_member_surface` records direct class/interface members with kind,
key/source spans, modifiers, parameter and generic-parameter spans, result and
initializer spans, and optional exact extracted-row slots. Named keys use
NameIds. Computed identifier/property paths retain lexical BindingIds or source
global NameIds plus selector NameIds; dynamic expressions and blocked scopes
remain unknown. Literal keys retain source spans pending semantic normalization.
No member slot is reconstructed from a name or qualified name.

`program_member_inputs` lowers surviving row slots to physical IDs and serializes
the source ID domains. `program_input::Part.surface` distinguishes absent evidence
from a known empty surface. Binding epoch 39 and extraction schema 59 invalidate
older source-ID allocations. Filtered source recapture, portable caches, poisoned
display names/signatures, and fresh-arena cold reload preserve this inventory.

These are syntax/signature **anchors**, not durable bound type recipes. The next
step must lower those type nodes while source is available, introduce semantic
signature/generic owners for members without navigation rows, bind unique-symbol
keys in the selected program, and check legal member/overload/index compatibility.
Cold resolution must not attempt to recover type syntax from display strings.

## Verification so far

- Initial capture tests failed (three tests); the first independent AST check
  rejected the `unique symbol` flag. The generated grammar aliases both tokens
  to `unique symbol`; the profile now records that exact sequence.
- Final source AST comparison: TypeScript 5.9.3 agrees on 10,237 member records in
  six authored fixtures and all 59 standard-library sources supplied by the
  pinned Query Core manifest. Checks include byte anchors, kinds, modifiers,
  parameters/rest/optional forms, generic parameter spans, results, initializers
  and unique-symbol syntax. This is not a target-binding or merge-legality oracle.
- Two separate compiler-diagnostic cases retain duplicate index signatures
  (TS2374 twice) and a legal index-plus-method control. Both stay incomplete in
  the engine until index-domain compatibility is implemented; no support claim
  is made for the legal control.
- Selected core/profile/oracle run: 1,959 passed, zero failed, 29 ignored.
- Source/ID budget audit: 301 production files and 99 ID-scoped files, zero
  violations. No public consumer-facing API changed.

The original and v2 inventory artifacts preserve the failed unique-symbol
observations. The v3 inventory is the compiler-validated capture. No artifact
was overwritten. Real-project non-interference is recorded below after rerun.

## Final non-interference and handoff

- All five selected integration targets passed (seven tests).
- Configured Query Core remains 436 correct, 40 declaration-kind-only
  disagreements, 214 unresolved out of 690 labelled calls: 63.1884% recall and
  91.5966% strict precision. The entire report equals the previous private-ID
  report except elapsed time; fresh and cold reports are deeply equal.
- Legacy remains 557 correct, 40 kind-only disagreements and 93 unresolved.
  Its entire report is likewise unchanged except elapsed time, fresh equals
  cold, and neither mode passes the independent 99% gate.
- Manifest population remains 187 supplied sources, 24 selected files and 843
  compiler calls / 690 labels. No source was edited or removed from the cohort.

Artifacts (SHA-256):

- `2026-09-08-configured-member-surface-inventory-v3.json`:
  `544eb27f7ab5ff164a13e41e2554c40e146dde669cf74274814a335abbd3f21d`.
- `2026-09-08-query-core-member-surface-program-report.json`:
  `a5f9d0c5792f062b3171a584c104358299ff9441d31c0f7adbae3f666a6d4976`.
- `2026-09-08-query-core-member-surface-legacy-report.json`:
  `cb017691f56917d35660a7f2f5b174f317f5b02c88349f68fb2b68ebe349ac8d`.

Only the source-inventory prerequisite is complete. F0-F3 and the parent
configured stdlib/initializer task remain open. Next: durable source signature
recipes and program-owned computed-key/merge compatibility, followed by the
initializer-derived types that the retained real traces show are still absent.
