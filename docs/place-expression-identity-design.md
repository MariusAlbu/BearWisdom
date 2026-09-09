# Source-owned place expression identities

1. [generic] Extend the existing ValueExpr recipe, not a parallel spelling-based resolver, with named fields, tuple positions and explicit dereferences.
2. [profile data] Supply field-node/selector and dereference-operator forms; the generic capture never branches on language names.
3. [generic] Intern field selectors during source ingestion and lower NameIds to compilation MemberNameIds once per file.
4. [generic] Carry exact expression spans into semantic call operands; arguments and local initializers evaluate the same recipes.
5. [generic] Read each operand BindingId at its own source position, without moving the active inference or module cursor.
6. [generic] Select nominal fields through the existing ID member index with source-module access and ambiguity barriers; substitute only GenericParamIds.
7. [generic] Preserve reference/pointer types in stored facts; implicit field projection is profile-controlled, explicit dereference removes exactly one attested indirection.
8. [generic] Keep unknown shapes, absent members, unsupported overloaded dereference and missing source recipes authoritative; never fall back to display strings.
9. [evidence] Add independent compiler target labels before implementation, then exact-ID, privacy, namesake, provider edit/deletion, filtered-cache and cold-reload tests.
10. [boundary] This slice does not establish move/borrow legality, arbitrary indexing, closure identity, pattern projection, complete inheritance substitution or the representative F0 99% gate.

## Implemented contract

`CallArg::ValueAt` carries an exact source expression span. Both argument operands
and local initializers evaluate the same `ValueExpr` tree: position-correct
BindingId reads, source-owned borrows, MemberNameId fields, tuple positions and
one-layer explicit dereferences. The generic evaluator uses canonical TypeIds,
member access at the selector byte and GenericParamId substitution; it does not
move the active inference/module cursor or recover identity from display text.
Rust supplies the initial syntax/profile data. Raw field identifiers are decoded
at declaration and occurrence ingestion, not repaired by runtime string matching.
Binding epoch is 26; extraction schema is 46.

## Evidence — 2026-09-07

- The original eleven new compiler-labelled cases improved from 19/38 to 38/38
  correct positive targets, closing nineteen misses with zero wrong targets.
- A separate direct raw-field control adds three targets. The final twelve-case
  cohort has 41/41 correct positives and one correct diagnostic-backed negative.
- Combined diagnostic corpus: 90 cases, 218/220 correct positives, two unresolved,
  zero wrong targets and 16 correct negatives; all 236 labelled occurrences are
  extracted and fresh/cold reports agree. Historical labels/snapshots are unchanged.
- Pinned rustc independently validates all 220 positive targets and 16 negatives.
  All twelve compiler-adapter tests pass. The seven parser tests also run directly
  under Node when the sandbox denies the test runner's child-process spawn.
- 2,163 selected core/profile tests and all twelve integrations passed. Exact-ID
  tests cover regions, source positions, privacy, raw pointers, generic fields,
  filtered portable caches, provider type/visibility edits, field/provider removal
  and fresh-arena cold reload. Forty-five fixture cases retain identical reports
  under poisoned annotation/display operands.
- AlphaT and Lynx checks stop before compilation: respectively a yanked locked
  `der` dependency and a required lockfile update. Neither lockfile was modified;
  downstream compatibility is not yet proven.

## Remaining boundaries

Nested call-result operands such as `&make().item`, closure-owned borrows,
pattern/destructuring projections, overloaded dereference, arbitrary indexing,
full region/coercion obligations and other language profiles remain unfinished.
The two retained diagnostic misses are enum match payloads. This authored cohort
is not representative multi-language evidence for the F0 99% correctness gate.
