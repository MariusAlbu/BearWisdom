# Exact source atomic types and literals

1. [generic] Keep unresolved engine evidence (`Type::Unknown`) distinct from language-level any/unknown/nullish/void/object types.
2. [generic] Add explicit intrinsic leaf identities and exact numeric/bigint/UTF-16 literal values to the canonical type arena, without nominal display-name keys.
3. [profile data] Decode predefined type tokens, literal CST shapes, numeric radix/separator/suffix rules and escape syntax at ingestion; no language-name resolver branch or API list.
4. [generic] Canonicalize literal values once: equivalent escapes/radices share identity, signed zero normalizes, arbitrary-size integers and lone UTF-16 surrogates remain lossless.
5. [generic] Carry intrinsic/literal recipes through lexical capture, durable configured signatures, generic substitution, arena remapping and cold snapshots.
6. [generic] Preserve unknown evidence for unsupported syntax; intrinsic identity is not member availability, assignability, generic applicability or merge legality.
7. [evidence] Start with failing source-capture tests, then independently compare captured intrinsic/literal facts with pinned TypeScript compiler types.
8. [evidence] Test escaped/non-BMP/lone-surrogate strings, numeric rounding/radix/large bigint cases, type-space namesakes and filtered/portable/configured snapshots.
9. [evidence] Run affected engine/core/oracle suites, exact per-occurrence real-project regression checks, and sibling-consumer checks for public type-algebra changes.
10. [scope] Continue toward operator recipes, selected-program computed-symbol keys, legal rich merging and initializer cascades; do not check off the parent F1/F0-F3 goals prematurely.

Root evidence: `lexical_type_syntax::expr` has no predefined/literal type forms;
`program_types::lower` turns the resulting Legacy payloads into Unknown. Thus
unrelated scalar signatures and unresolved syntax lose distinct type identities
before any legal merge checker could compare them. Recovering them from display
text at resolution time would violate the ID-based design.

## Receiver projection dependency exposed by integration

1. [evidence] The unchanged integration corpus loses both String method targets after intrinsic capture; the old path capitalized a nominal string at lookup time.
2. [profile data] Describe intrinsic-to-global-wrapper associations as source ingestion data, not member API names or runtime capitalization.
3. [generic] Intern wrapper names once into each source NameId arena and persist intrinsic-to-NameId recipes.
4. [generic] Bind configured wrapper declarations using the selected program's existing global groups and completeness barriers.
5. [generic] Prebind unconfigured wrapper declarations once per compilation from complete, non-isolated source-global declarations; reject competing owners.
6. [generic] Expose only intrinsic-to-TypeId receiver projection to semantic traversal; never reconstruct names from type display.
7. [generic] Project only member receivers; parameter, field, literal and return identities remain unchanged.
8. [generic] Apply the same projection to configured method selection and ordinary chain walking, with missing/ambiguous providers authoritative.
9. [evidence] Retain the original integration assertions, and test namesakes, missing providers, fresh/cold persistence and configured selection.
10. [scope] Actual rich merges, initializer inference, type operators and computed-symbol binding remain open; boxing is not blanket stdlib acceptance.

## Verified implementation — 2026-09-08

- `Intrinsic` supplies twelve canonical leaf identities, distinct from unresolved
  `Type::Unknown` and nominal declarations. `LitValue` retains IEEE binary64 bits,
  arbitrary-size signed bigint words and unpaired UTF-16 code units. Equal numeric
  radices/escapes share values; negative zero is normalized. Numeric ingestion is
  bounded at 16,384 source bytes and unsupported evidence stays unknown.
- Profile-owned atomic forms feed lexical capture, `TypeBinder`, durable program
  recipes, physical/source-only signatures, generic substitution, portable parse
  types and arena snapshots. Binding epoch is 42; extraction schema is 62.
- The initial regression exposed `Legacy("string")`. Broader independently
  labelled coverage then exposed `bigint` as a `type_identifier`; the pinned
  grammar now recognizes it as `predefined_type` in both dialects while keeping
  ordinary `bigint` value bindings and the distinct `BigInt` type name intact.
- Receiver projection uses profile wrapper NameIds bound to selected-program
  global declaration IDs. The unconfigured environment prebinds unambiguous
  complete script-global owners once per compilation. It excludes module/local
  namesakes and rejects competing owners; it does not admit rich merges.
  Exact source/literal types are unchanged by member-only projection. No runtime
  capitalization, display parsing or additional language hook was introduced.
- The original integration corpus caught two lost String method targets after
  atomic capture. The ID projection repairs both without changing assertions.

Verification: 1,982 selected library tests passed, 29 ignored; eight pinned-grammar
tests and all seven selected integration tests passed. The final broad run used
the current compiled lib-test binary after the targeted Cargo build. Independent
TypeScript 5.9.3 checks validate all 49 shared atomic labels (both TS/TSX ingestion
paths), nineteen existing generic-owner uses and fourteen grammar AST controls.
Configured tests cover filtered/portable source restoration, exact fields and
constraints/defaults, absent versus unresolved versus language unknown, poisoned
display inputs, actual signature edits and new-arena cold snapshots. Wrapper
tests cover configured/unconfigured selection, source/literal type preservation,
global versus local/module namesakes and missing providers, fresh and cold.

The fixed 843-call / 690-label Query Core population and source manifest were not
changed. Final reports are identical to the source-signature baseline per
occurrence and in metadata, except elapsed time; fresh equals cold in each:

- `resolution-documents/2026-09-08-query-core-atomic-wrapper-program-report.json`:
  436 correct, 40 kind-only disagreements, 214 unresolved; SHA-256
  `e215e674213535113eacd62971f4b908b23ef1d0ffce6048e812eea231136de3`.
- `resolution-documents/2026-09-08-query-core-atomic-wrapper-legacy-report.json`:
  557 correct, 40 kind-only disagreements, 93 unresolved; SHA-256
  `7ac429fed40e6dc8e7e33f9e78f11466f4f1b45dbbfdb439d4b407c80cd7f17f`.
- Final member inventory `2026-09-08-configured-atomic-wrapper-member-inventory.json`
  still matches all 10,237 compiler AST member surfaces across 65 files. This is
  syntax evidence only, not successful merge compatibility or call binding.
- Earlier `atomic-type-*` reports and inventories remain as pre-projection
  evidence. No reports were overwritten. Timings were not controlled benchmarks.
- Native audits: 306 production files with zero budget violations; 102 identity
  files with zero prohibited string-pattern growth; `git diff --check` passed.

Consumer verification remains incomplete. AlphaT's offline locked check stopped
at yanked `der 0.8.0` dependency resolution; an online locked retry refreshed
registry metadata and then required a lockfile update. Lynx's offline locked
check also requires a lockfile update. Neither reached consumer compilation;
neither lockfile nor consumer source was changed. Dependency refresh authority
is separate from continuing the engine work.

Remaining F1 work is unchanged in scope: exact operator/composite recipes,
selected-program computed unique-symbol keys, rich property/index/call/construct
and overload/generic compatibility, then initializer-derived field/value
cascades. Apparent receiver binding must use that same legality evidence as rich
merges expand. F0 representative 99%/99.9% proof, the other language/ID/configuration
work, F2 inference/snapshots/IDE APIs and F3 product/flow benchmarks remain open.
