# Pattern payload identities

1. [generic] Treat pattern-bound values as source-owned value recipes, not annotation strings or whole-scrutinee copies.
2. [profile data] Configure arm scopes, scrutinee/arm fields, pattern constructors and payload declaration shapes.
3. [generic] Declare leaves in their arm ScopeId before capturing reads; sibling arms and outer namesakes retain distinct BindingIds.
4. [profile data] Emit physical enum payload fields, including positional names at ingestion, beneath their variant declaration slots.
5. [generic] Capture payload TypeExpr recipes at exact field source positions and preserve existing field-ID persistence/filtering.
6. [generic] Bind each pattern's qualified enum head once and lower its variant/field NameIds to MemberNameIds.
7. [generic] Evaluate the scrutinee at its source position, attest canonical enum identity, select exact variant/field declaration IDs and substitute enum GenericParamIds.
8. [generic] Preserve missing/ambiguous/inaccessible evidence; unsupported reference binding modes and pattern shapes cannot borrow outer namesake types.
9. [evidence] Preserve the two historical compiler-labelled match failures, add independent cascade/generic/import/negative fixtures, and require exact fresh/cold targets.
10. [boundary] This step does not complete pattern exhaustiveness, match ergonomics, or-pattern equality, arbitrary scrutinee expressions, CFG joins or the representative F0 gate.

## Implemented contract — 2026-09-07

Rust profile data now supplies match-arm scopes, constructor/field pattern shapes,
guard boundaries and positional payload declaration forms. Leaves are declared
with arm-local BindingIds and become available before the guard expression.
Unsupported captured patterns own Unknown facts, rather than inheriting outer
namesake types. Or-pattern leaves are captured but their equivalence is not solved.

Enum variants retain physical parent IDs. Named and positional payload fields
are now emitted beneath those variants, with source-coordinate TypeExpr capture
and ordinary field-TypeId persistence. Positional member names and raw identifier
decoding belong to ingestion only. The extraction refactor preserves existing
variant type-reference and attribute emission.

`ValueExpr::VariantField` carries variant/field NameIds, an exact pattern selector
byte and a recursive scrutinee recipe. Per-file lowering produces MemberNameIds;
the selector's separately captured enum type recipe is lowered to a TypeId.
The evaluator requires matching canonical enum IDs, exact fixed alias application
identity where applicable, unique accessible variant/field IDs and enum-owner
GenericParamId substitution. Nested owned enum patterns compose this same recipe.
The source-position read does not move the active inference/module cursor.

Reference scrutinees, raw pointers, explicit ref binding modes, positional rest
and unsupported shapes remain Unknown. This does not silently reinterpret a
borrowed payload as owned, or claim full Rust pattern/type-checker semantics.
Binding epoch is 27; extraction schema is 47.

## Verification

- The first new return/local cascade failed at 0/4 correct targets before the
  implementation and now passes 4/4. Both historical match payload misses also
  resolve and are promoted to strict regression gates; gold labels and historical
  observation snapshots remain unchanged.
- Seven new compiler-validated cases have sixteen correct positive targets and
  one correct negative. Cases cover positional/named/nested/generic patterns,
  imported enum aliases, cross-file namesakes, sibling/outer isolation and guards.
- Full diagnostic report: 97 cases, 236/236 correct positives, 17/17 correct
  negatives, zero wrong/unresolved/missing/unlabelled occurrences. All 253 labelled
  occurrences are extracted; fresh/cold results agree.
- Pinned rustc 1.94.0 independently verifies all ten fixture files. All twelve
  compiler-adapter tests pass. Fifty-two argument/qualified/borrow/local/place/
  pattern cases retain identical reports under poisoned annotation/display data.
- 2,170 selected core/profile tests and twelve integrations pass. Dedicated tests
  cover exact nominal TypeIds, unsupported binding-mode barriers, provider payload
  type/access edits, payload/provider removal, filtered portable caches and
  fresh-arena cold reload. The production-file budget/ID-pattern audit passes.

This is an authored diagnostic cohort, not representative per-language evidence
for the F0 99% recall / 99.9% precision gate. General reference-pattern semantics,
other destructuring forms, closures, nested call operands and F2/F3 remain open.
