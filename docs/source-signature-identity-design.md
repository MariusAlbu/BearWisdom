# Source-owned signature types

1. [generic] Preserve signature identity as a source-span ID, independent of optional navigation rows and scoped to a validated configured source instance.
2. [profile data] Give call/construct/abstract method signatures their own lexical generic scopes; no language-name branches in resolution.
3. [generic] Capture parameter/result type recipes and generic constraints/defaults while CST/source are available, including nested function-type binders.
4. [generic] Use source signature IDs for rowless generic owners; retain exact physical declaration IDs where present and never correlate by display spelling.
5. [generic] Persist the new source signature arena with the configured type inputs; invalidate both binding and extraction cache epochs.
6. [generic] Allocate source-owned GenericParamIds in each program view before materializing signatures; keep physical/source owner correspondence explicit.
7. [generic] Materialize all supported signature type recipes through the selected program's declaration/import/global IDs; unsupported types remain unknown, never reparsed from display strings.
8. [generic] Expose bound source signatures independently of navigation, retaining optional/rest flags, constraints/defaults and absent result annotations for later compatibility/overload proofs.
9. [evidence] Independently compiler-check scoped generic uses; test no-row, filtered, portable, poisoned-display, overlapping-program and cold-source cases before claiming this dependency complete.
10. [evidence] Remeasure the pinned real cohort and retain negative barriers; unique-symbol binding, rich merge legality, initializer inference and all F0-F3 parent goals remain required.

## Root evidence

The retained Query Core traces show rejected standard-library type groups and an
untyped initializer-derived `Subscribable.listeners` field. Existing member
capture preserves spans but cannot provide semantic signatures for merge checks.
`lexical_type_syntax::expr` currently records `Parameter { owner: None }` when no
navigation row exists. The TypeScript profile also omits call/construct generic
scopes, allowing sibling signatures' type parameters to occupy one environment.
These are source identity failures upstream of safe standard-library merging.

## Implemented and verified

`lexical_signature_types` captures a source `SignatureId` independently of an
optional physical row, parameter/result recipes, generic constraints/defaults,
and source optional/rest/type-parameter evidence. Generic bindings retain their
source owner even without a row. The TypeScript profile now creates separate
call/construct/abstract-method scopes and decodes constraint/default wrappers.

`program_signature_types` persists these inputs and allocates source-owned
generic parameters inside each configured view before materializing types.
Physical generic owners retain their selected declaration's IDs. Nested function
types can refer to their own rowless generics and to enclosing physical generics
without borrowing either binder's spelling. Bound constraints/defaults are
evidence for future legality checks, not proof those checks have passed.

The internal lookup surface exposes source-signature generic IDs, and FileLookup
forwards them only through the selected source snapshot. Binding epoch 40 and
extraction schema 60 invalidate prior source allocations. Configured type inputs
now have deterministic persistence ordering. No public consumer API changed.

Verification:

- The initial scope regression failed: sibling call/construct signatures both
  had `BindingId(1)`. It passes after the profile scope correction.
- TypeScript 5.9.3 independently checks 19 generic-use labels across three
  fixtures, including three out-of-scope negatives and UTF-8 source anchors.
- New tests cover rowless ownership, nested generic binders, constraints/defaults,
  optional/rest flags, filtered/portable providers, poisoned display metadata,
  source/program isolation, late providers, actual barrel edits, deletion and
  fresh-arena cold reload. The provider test deliberately filters unnamed
  navigation rows through the normal remapping path; ordinary extraction may
  emit those rows, so the test does not claim they are always absent.
- Selected engine/profile/oracle suite: 1,968 passed, zero failures, 29 ignored.
- The regenerated 10,237-member inventory still agrees with the independent
  compiler AST verifier; its SHA-256 equals the prior v3 inventory.
- Production file-budget audit: 304 files, zero violations. ID-pattern growth
  audit: 100 files, zero violations. Diff whitespace check passed.

## Remaining dependency chain

The source-signature arena is implemented; complete type syntax and legal rich
merging are not. Existing supported declaration/import/global/generic/application/
function/tuple/union/intersection/optional recipes materialize without string
recovery. Other forms still lower to Unknown, including predefined scalar forms
not yet classified by the TS lexical type profile. The source `unique_symbol`
flag is still syntax, not a selected-program symbol identity. No merge guard was
relaxed, no overload was selected and no generic constraint was silently accepted.

Next, complete exact intrinsic/literal/operator type recipes and bind computed
keys through selected-program value/member identities, then check overload,
property/index and generic compatibility before admitting standard-library
groups. Follow with initializer-derived field/value types and the retained
Set/Promise/Array/listeners cascade remeasurement. All F0-F3 goals remain active.

## Real-project non-interference and closeout

- All five selected integration targets passed (seven tests).
- Configured Query Core remains 436 correct / 40 declaration-kind-only
  disagreements / 214 unresolved on the unchanged 690 labelled calls.
- Legacy remains 557 correct / 40 kind-only disagreements / 93 unresolved.
- Both complete reports equal their prior member-surface reports except elapsed
  time, and both have deep-equal fresh/cold results. Neither passes the 99% gate.
- The pinned 187-source / 24-selected-file / 843-call manifest is unchanged.

Retained artifacts (SHA-256):

- `2026-09-08-query-core-source-signature-program-report.json`:
  `d789299ae86ee6562f71e2369567a015220b39d2952a24d0d9a60fbaf50f2f6d`.
- `2026-09-08-query-core-source-signature-legacy-report.json`:
  `f7b6413fac6339625c9582e207636700126a8c5a5001a6030f85fde7a3f2e29e`.
- `2026-09-08-configured-source-signature-member-inventory.json`:
  `544eb27f7ab5ff164a13e41e2554c40e146dde669cf74274814a335abbd3f21d`.

No prior artifact was overwritten. Only the source-signature ownership
prerequisite is checked off; the parent type/key/merge/initializer task is open.
