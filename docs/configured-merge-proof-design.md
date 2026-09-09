# Configured declaration merge proof

1. Keep source/program declaration candidates separate from publicly admitted groups in program_graph.
2. Retain header completeness independently from the old plain-member flag; unsupported heritage is not erased.
3. Allocate a private candidate view using canonical declaration/generic IDs and existing source signature/query recipes.
4. Validate member keys and cross-part property, method, call, constructor and index compatibility using bound IDs.
5. Decode profile syntax and intern member names only at ingestion; add no runtime name recovery or builtin lists.
6. Keep engine Unknown distinct from language-level top types; equality of unresolved operands is never proof.
7. Remove failed candidate groups and rebuild/revalidate dependents until admission stabilizes; publish no intermediate view.
8. Keep unsupported owner constraints, heritage, accessors, literal keys and unique-origin canonicalization explicit until their proofs exist.
9. Verify compiler-labelled call cascades, incompatible and dependency-invalidated groups, fresh/cold/edit/deletion and source isolation.
10. Remeasure every retained Query Core occurrence; then continue initializer-derived types and the full F0–F3 roadmap.

## Repeated-property projection (2026-09-08)

The expanded compiler fixture proves `list.value.touch()` should bind after the
two property declarations have been proved equivalent. Admission alone left
their physical rows ambiguous in `member_selection`. Return numeric property
equivalence classes from compatibility checking; only the final stable view
projects those rows to one semantic property owner and installs its proved
field type. Keep all physical rows for navigation. Do not collapse overloads,
unproved properties, or unrelated namesakes; source-unique origin unification
remains a separate proof obligation.

## Implemented boundary

`program_merge_candidates` retains eligible interface-only candidates behind
the original graph's rejection barrier. Owner parameter names have already
been interned into numeric program IDs; only matching plain parameter/header
forms with source member inventories enter staging. Heritage and constraints
are not silently dropped. `plain_header` is durable ingestion evidence
(binding epoch 47 / extractor schema 67); missing old evidence fails closed.

`program_merge_proof` builds an isolated candidate view, checks source-bound
signature/key facts, removes failures, and rebuilds until all remaining
candidates pass. No intermediate view is published. Rejection can propagate
through computed keys, annotated global values, aliases and nominal types.
Unrelated groups survive, and subsequent snapshots may admit a repaired group.
The original graph remains unproved; configured runtime consumers use only the
final selected view. No mutable admission proofs are serialized.

Member key domains distinguish interned names, unique-symbol TypeIds, call,
construct and index signatures. Repeated properties require compatible
modifiers and proved equal types; repeated methods retain overload parts.
Duplicate index domains and incompatible string/symbol index members reject
the group. String indexes also constrain number-index result types. Literal
and numeric-name keys remain unsupported rather than escaping index checks.

`program_merge_types` compares canonical bound IDs, including numeric alias
expansion and supported recursive type shapes. Unresolved `Type::Unknown`
never proves equality, even to itself; language `unknown` is a distinct
intrinsic type. Optional sources distribute before target-union choice.
These are conservative cross-declaration compatibility checks, not a complete
type checker: disjoint signatures need no equality proof, and unresolved
signature results remain unresolved downstream.

```typescript
interface Registry<T> { [index: number]: T; }
interface Registry<T> { first(): T; forEach(f: (value: T) => void): void; }
declare const list: Registry<Doc>;
list.first().touch(); // ✅ [generic] staged owner + signature generic IDs
list.forEach(value => value.touch()); // ✅ [generic] contextual return cascade

interface Catalog { value: Doc; }
interface Catalog { value: Item; } // Item is a bound alias to Doc
declare const catalog: Catalog;
catalog.value.touch(); // ✅ [generic] proved property-equivalence projection

interface Bad { value: string; }
interface Bad { value: number; } // ✅ [generic] conflicting group rejected
interface Pending { readonly tag: unique symbol; }
interface Pending { readonly tag: unique symbol; }
// ⚠️ [generic] source-unique origins need a separate merged-identity proof
```

## Retained limitations

Constrained/defaulted or alpha-renamed owner parameters, interface heritage,
rich class/interface and namespace/entity merges, accessors, literal member
keys, structural function compatibility, general subtyping/variance and type
operator evaluation remain incomplete. Computed-key dependencies on repeated
properties still need property equivalence during private staging; final
runtime projection alone does not prove those query dependencies. Full
unique-origin canonicalization is not implemented. Overload retention is not
overload selection. Initializer-derived field/value recipes remain next work.

The shrinking solver is correctness-first: it clones/builds views per round,
with at most candidate-count + 1 rounds. It does not yet maintain a sparse
dependency worklist. Uncontrolled project-run timings are not performance or
competitive benchmark evidence.

## Verification and retained project evidence

- Reproduced the optional index compatibility failure before changing the
  relation; all 16 shared compiler-diagnostic fixtures now pass fresh/cold:
  three admitted, eight diagnostic-backed rejections, five legal unsupported.
- Six compiler-validated multi-file fixtures contain 15 exact call targets,
  including callback/return cascades, dependent computed keys and repeated
  property aliases. `verify_modules.mjs` checks TypeScript 5.9.3 targets;
  configured engine tests check the same labels fresh/cold and poisoned display.
- Expanded `list.value.touch()` failed after admission alone; property
  equivalence projection closes it without choosing an overload target.
- Provider edit makes the duplicate property incompatible, removes its
  computed-key dependent, and leaves an independent merge available. Cold
  reload agrees. Deleting the conflicting provider requires refreshed source
  membership before readmission. A separate compiler check accepted the valid
  and deleted programs and emitted diagnostic 2717 for the conflicting program.
- Rowless contract/portable recipes, program overlap, stale source hashes,
  generic property identity, overload ambiguity and unrelated namesakes are
  covered by focused tests.
- Final selected library suite: 2,022 passed, 29 ignored, zero failed; seven
  integrations passed across resolution_corpus, resolution_corpus_js,
  resolution_corpus_rust, per_file_manifest and per_package_context.
- File-budget checks on all nine touched production modules and
  `git -c core.safecrlf=false diff --check` passed.

Unchanged compiler manifest:
`resolution-documents/2026-09-07-query-core-configured-compiler-manifest.json`
SHA-256 `44158824a3f91fdeda2d5e5da8a422054e178c76de29857c2f855ffe36a5110c`.
187 supplied sources, 24 selected files, 843 compiler calls, 690 labels;
153 compiler calls remain unlabelled by this adapter, not counted as correct.

Final configured report:
`resolution-documents/2026-09-08-query-core-merge-property-program-report.json`
SHA-256 `a9bcf495925f1c9e6c7501222a4b10af29acb2d0e3f400d6061bcb3092e15ca9`.
446 correct, 40 unchanged declaration-kind disagreements, 204 unresolved;
64.637681% strict recall, 91.769547% strict precision, zero source-capture gaps,
zero fresh/cold changes, gate ineligible. All ten changed labelled occurrences
improved from unresolved to correct: seven Date.now, two Object.defineProperty
and one Object.getPrototypeOf. No gold labels or observation population changed.
Elapsed 32,271 ms is uncontrolled timing, not a speed claim.

The earlier admission-only report is retained at
`resolution-documents/2026-09-08-query-core-merge-proof-program-report.json`
(SHA-256 `a49bd58d6673b97f5f6a08d9a83e47abaa6af4cf547c8c546b17b0ef93ceadf6`).
Its entire JSON equals the final report except elapsed time; property projection
improves the authored cascade but does not add further real-cohort gains yet.

The read-only real-program diagnostic now distinguishes the original fenced
graph from final selected-view admission. Array/ReadonlyArray still include
unsupported heritage in Node's compatibility/indexable provider. Set, Map,
Promise and PromiseConstructor type groups also remain unadmitted and require
tracing their first missing compatibility/key evidence. Their separate global
value declarations being visible is not evidence their type groups were proved.
`Subscribable.listeners` still has no configured initializer-derived field type.
Do not mark the parent stdlib/initializer task, F1, or the full 99% goal complete.

Final legacy control:
`resolution-documents/2026-09-08-query-core-merge-property-legacy-report.json`
SHA-256 `6c3778ad5933b6dab02ee889c87e355ee920f3f31dd4d60a323e4a57e7c06571`.
557 correct, 40 kind-only disagreements, 93 unresolved; 80.724638% strict recall,
93.299832% strict precision, zero snapshot changes, gate ineligible. Its whole
JSON is identical to the source-value-query legacy report except elapsed time
(42,591 ms). No production changes followed the final verification runs.
