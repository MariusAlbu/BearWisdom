# Source-owned interface inheritance

1. Keep class value-head recipes separate from interface type-domain base recipes; do not reinterpret runtime bindings as types.
2. Capture every source interface, including module-local and nested declarations, using profile-declared heritage syntax and exact declaration/signature anchors.
3. Persist base applications and complete member inventories while source is available; declaration, binding, generic and member IDs own semantic relationships.
4. Stage recognized interface headers privately; syntax recognition is not permission to publish an unproved merged group.
5. Bind multiple bases in the selected program, reject missing/foreign/cyclic heads and incompatible generic applications, and preserve class single-parent behavior.
6. Substitute source-owned generic parameters along inherited paths and prove own/inherited member compatibility before admitting inheritance.
7. Share proved numeric parent edges with member selection and generic yields; raw value-query evaluation must not read partially installed tables.
8. Propagate failed heritage proofs through dependent declarations and rebuild private views until admission stabilizes; never recover a namesake by spelling.
9. Verify compiler-labelled positive cascades and diagnostic-backed negatives, then fresh/cold, portable/filtered, edited/deleted and overlapping-program controls.
10. Remeasure all retained Query Core occurrences without changing labels; initializer inference, complete structural/type-operator semantics and the full F0–F3 goal remain open.

## Structural and mapped base work (2026-09-08, in progress)

1. The current report has 546/690 correct, 104 unresolved and 40 kind-only disagreements; fresh/cold are equal. Compared with the 492 baseline there are 56 gains and two regressions, not a regression-free closeout.
2. Both regressions belong to QueryObserverOptions, whose WithRequired base combines a nominal application with a mapped structural type. Preserve that failure until the source semantics are represented and proved.
3. Add a generic structural type algebra with ID-valued property keys/values and mapped binder/key/remap/value operands; source literals are interned at the boundary, never used to re-find declarations.
4. Capture object and mapped syntax through profile data. Give each mapped parameter a source-owned lexical BindingId and row-independent signature owner, including nested shadowing. Compiler correction: the binder is in scope in its own key constraint; `[K in K]` is a circular constraint, not a reference to an outer namesake.
5. Persist and reconstruct those recipes in the selected source/program; generic substitution, context checks and portable/cold remapping must visit every operand and preserve binder ownership.
6. Keep unsupported members, modifiers and erroneous syntax authoritative Unknown; an empty object is a real structural type, not an engine bailout or a nominal placeholder.
7. Evaluate finite mapped keys and structural/intersection member surfaces from bound types. Preserve optionality, readonly, index domains and provenance rather than erasing the structural branch of an intersection.
8. Extend private inheritance proofs to those evaluated surfaces and generic obligations before projecting members or publishing nominal ancestor paths.
9. Add independent compiler-positive and diagnostic-negative fixtures before implementation, then exact targets, poisoned-display, import edits/deletion and portable/cold controls.
10. Rerun the selected library/integration tests and every real-project occurrence. Do not finish the compound roadmap task while repeated-property dependencies, initializer inference or either regression remains open.

```ts
interface RelativeIndexable<T> { at(index: number): T | undefined; }
interface Array<T> extends RelativeIndexable<T> {}
// ✅ [generic] Source base declaration IDs, generic substitution and compatible
// inherited member projection now pass the compiler-labelled inheritance cohort.
// ✅ [profile data] Interface clauses select type syntax and declared-base ordering.

interface Invalid extends Left<string>, Left<number> {}
// ✅ [generic] Conflicting generic paths are rejected by the private surface proof.
```

## Implemented source structure (not evaluation)

`TypeOperator::Object` retains ID-valued property keys, values, optional/readonly
flags and index-domain identity. `TypeOperator::Mapped` retains a source-owned
generic binder plus keys, remap and value operands; absent/add/remove modifiers
stay distinct. The existing recursive recipe, arena-context, snapshot and
portable-cache machinery visits all of these operands. The operand iterator
does not allocate a vector just to enumerate a structural type's children.

`lexical_structural_types.rs` captures supported property/index/mapped syntax
through the TypeScript profile. Mapped binders get lexical BindingIds and
row-independent source SignatureIds. Unsupported method/construct/computed
object members remain Unknown rather than becoming an empty object. Escaped
identifier keys are also still unsupported; decoded literal string keys are
supported. This is not complete callable/structural source semantics.

```ts
type Wrap<T, K extends keyof T> = { [P in K as P]: T[P] };
// ✅ [generic] Distinct source binder IDs survive substitution and cold hydration.
// ✅ [profile data] Mapped clause/annotation syntax supplies remap and modifier axes.

type Circular<T, K extends keyof T> = { [K in K]: T[K] };
// ✅ [generic] Capture binds both K uses to the inner declaration; it does not
// borrow the outer K. The compiler reports 2313/2536; this is not a legal proof.

type WithRequired<T, K extends keyof T> = T & { [P in K]: {} };
// ✅ [generic] Source recipes retain both intersection operands and mapped IDs.
// ⚠️ [generic] Supported finite structural evaluation now exists; mixed nominal
// surfaces and complete generic obligations still block this inheritance proof.
// See structural-type-evaluation-design.md for the current implementation.
```

Compiler evidence: `structural_type_fixtures.json` is shared by source-capture
and configured-materialization tests. TypeScript 5.9.3 independently verified
10 shapes, 10 mapped owners and 28 exact generic-use targets, including four
diagnostics across two circular-constraint negatives. The initial engine test
failed on `Legacy("{}")`; a separate substitution test failed when the mapped
binder was replaced. Both now pass. These checks do not prove mapped evaluation
or navigation target completeness.

At this source-capture checkpoint, cache versions were binding epoch 50 and extraction schema 70. The
selected library verification passed 2,040 tests, with 30 existing ignored tests.
Tests cover filtered/portable providers, poisoned display metadata, imported
operand retargeting/deletion, source-local binder separation and fresh-arena cold
reload. The full 99% gate and the compound roadmap inheritance task remain open.

All seven selected integrations also pass. The new configured report
`resolution-documents/2026-09-08-query-core-structural-recipes-program-report.json`
is identical to the prior interface-defaults report except elapsed time: all 690
fresh and 690 cold occurrences, labels, metadata and kind disagreements are
unchanged. Counts remain 546 correct / 40 kind-only / 104 unresolved; recall
79.1304347826%, strict precision 93.1740614334%, zero snapshot changes and gate
false. Report SHA-256: `ccc01baf4336b39c45715b0ce74829f7d139418d0855f97220602bfeb6c5f4b6`.
The unchanged compiler manifest SHA-256 remains
`44158824a3f91fdeda2d5e5da8a422054e178c76de29857c2f855ffe36a5110c`.
Elapsed times are uncontrolled diagnostic runtimes, not performance evidence.

The previous inheritance slice's net gain remains 54 calls: 56 unresolved-to-correct
improvements minus two correct-to-unresolved regressions at queryObserver.ts byte
10706 (`refetchInterval`) and 15174 (`select`). Neither regression is repaired by
source recipe capture alone. The real base also supplies QueryOptions generic
arguments constrained by QueryKey, whose source contains conditional/infer
syntax; inspect the next failed proof instead of assuming mapped expansion alone
will complete generic legality.

The latest legacy report is also unchanged in its entirety except elapsed time
against the value-domain baseline: 557 correct / 40 kind-only / 93 unresolved,
fresh/cold equal, gate false. File:
`resolution-documents/2026-09-08-query-core-structural-recipes-legacy-report.json`;
SHA-256 `2123536ccb5223db932dbdde4db653700627f9c0efa79ffd786193f91c5ed63e`.
Native file-budget and ID-pattern audits passed (309 and 111 production sources
respectively); `git diff --check` passed. AlphaT/Lynx dependency/lockfile compilation
verification remains outstanding and their lockfiles were not changed.
