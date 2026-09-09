# Selected array identities and readonly constructor inference

1. Keep the retained Set constructor-to-callback cascade as the real-project target; readonly canonicalization is its first proven barrier.
2. Declare mutable/readonly array global roles as TypeScript grammar profile data, not resolver library-name tests.
3. Capture those role requests into NameIds and persist them with configured program inputs; bump extractor schema and binding epoch.
4. Bind the roles to selected-program canonical declaration IDs, checking agreement and generic arity; local/imported namesakes cannot inherit a global role.
5. Preserve the existing nominal Apply representation and generic arguments; readonly arrays canonicalize to the selected readonly-array declaration, never an erased mutable type.
6. Admit readonly syntax only on source array/tuple operands; preserve readonly tuple structure and reject illegal alias/object/operator operands.
7. Share array element inference across attested mutable/readonly array heads; propagate ordinary GenericParamId candidates, never choose types by display metadata.
8. Prove array argument relations with element-type evidence and mutable/readonly directionality in both assignment and subtype phases; unsupported tuple/index/variance cases remain unknown.
9. Add independently compiler-labelled result/diagnostic fixtures and exact source-ID local/inline cascade tests with portable/cold/configuration/namesake negatives.
10. Remeasure the fixed Query Core cohort and keep iterable inference, constructor selected-origin/order evidence and callback negation explicit until the whole real cascade is proved.

```ts
new Build(array);             // ✅ [generic] attested array heads contribute element candidates to GenericParamIds.
new Build(readonlyArray);     // ✅ [generic] readonly syntax normalizes to the selected readonly declaration.
takeMutable(readonlyArray);   // ✅ [generic] array relation rejects readonly-to-mutable conversion.
localArrayNamedTwin.member(); // ✅ [generic] local namesakes do not acquire selected global array roles.
```

## Implementation and evidence

`lexical_array_types` owns grammar profile role requests and readonly operand
validation. The TypeScript profile supplies `Array`/`ReadonlyArray` spellings at
ingestion; persisted `NameId` requests bind to canonical declaration IDs in each
selected view. Both roles require source interface evidence and one generic
parameter. Conflicts, missing providers, non-interface declarations and foreign
nominal contexts cannot confer a role. No library/member-name resolver hook was
added. Binding epoch is 64; external parse schema is 78.

`program_array_types` shares role/element evidence across constructor inference,
assignment, subtype and callable relations. Container compatibility is not an
element proof: callers must prove elements in the selected language phase. The
constructor still checks final arguments and generic constraints after inference.
Readonly tuple normalization preserves optional-slot arity instead of collapsing
`readonly [string?]` into `readonly [string | undefined]`. General tuple-to-array
inference and readonly-tuple structural operations remain unsupported.

The new 12-case array constructor cohort includes six supported positives, five
compiler-diagnostic negatives and one compiler-legal tuple abstention. Its engine
test uses shifted portable arenas, filtered cached contracts, poisoned symbol
names/qnames/signatures and cold reloads. A separate full source-member cascade
checks exact declaration IDs for both a local result and an inline chained result.
Configuration-only provider switching, provider edits/deletion, foreign contexts,
wrong generic arity and class namesakes have explicit controls.

The lifecycle test initially passed the unnormalized readonly annotation directly
to the role recognizer. The corrected test follows the production boundary:
canonicalize source type syntax before asking for a nominal array shape. It still
asserts readonly directionality and exact selected declaration identity.

## Real-project result

The frozen 187-source / 24-selected-file Query Core population is unchanged:
843 compiler calls, 690 independent target labels, 153 unlabelled calls. Neither
report changes at any field other than elapsed time relative to the preceding
constructor-union reports. There is **no measured recall gain** from this step.

- Configured: 575 correct, 40 declaration-kind-only disagreements, 75 unresolved;
  83.3333% recall, 93.4959% strict precision, zero fresh/cold changes.
- Legacy: 557 correct, 40 kind-only disagreements, 93 unresolved; zero fresh/cold changes.
- Configured report: `resolution-documents/2026-09-08-query-core-array-identities-verified-program-report.json`, SHA-256 `3adb7b7eed0c85f650ec5c80709ef81dff8a8df522605e2f54e0e296c575613c`.
- Legacy report: `resolution-documents/2026-09-08-query-core-array-identities-verified-legacy-report.json`, SHA-256 `03e68a885fe03edc2e6edd62f5f9fc2dad9a6e05d383ec422298c841a2b070c7`.
- Manifest SHA-256 remains `44158824a3f91fdeda2d5e5da8a422054e178c76de29857c2f855ffe36a5110c`.

Elapsed times (53,577 ms configured / 65,068 ms legacy) are diagnostic, not a
controlled performance comparison; the legacy run overlapped test compilation.

## Retained root and next work

```ts
const excludeSet = new Set(array2);
// ✅ [generic] source callee and caller-owned Array<T> argument are bound.
// ✅ [generic] collection constructor readonly T[] | null now proves Set<T>.
// ❌ [generic] iterable constructor applicability remains unknown; result is withheld.
array1.filter(x => !excludeSet.has(x));
// ❌ [generic] full cascade still needs the constructor result and callback negation.
```

The extended source-provenance probe identifies an earlier dependency inside the
iterable case: `Array<T>` and `Iterable<T>` agree on the exact computed unique-symbol
key, but the array's iterator signature returns `unknown<T>` in the selected view.
The source declaration is `ArrayIterator<T>`; trace its admission/heritage before
implementing a shallow structural key match. The target iterator signatures retain
their return, optional and tuple-rest information. Matching the computed key alone
cannot prove their compatibility.

The final probe follows this to two concrete source barriers:

```ts
interface IteratorObject<T, TReturn = unknown, TNext = unknown> extends Iterator<T, TReturn, TNext> {}
interface IteratorObject<T, TReturn, TNext> {} // Node compatibility augmentation
// ❌ [generic] program_merge_candidates::capture rejects non-plain generic parameters
// before a private compatibility proof can reconcile omitted versus supplied defaults.
type BuiltinIteratorReturn = intrinsic;
// ❌ [profile data + generic] captured as a global name recipe, not a configured compiler intrinsic.
```

The selected view rejects both physical `IteratorObject` declarations (14167 and
17126 in this probe) and `ArrayIterator` (14170). The global intrinsic alias (14169)
is present but has a `Global(NameId)` alias recipe. The NodeJS namespace also owns
a different, conditional alias of the same spelling; they must stay distinct.
Pinned TypeScript 5.9.3 `areTypeParametersIdentical` compares constraints/defaults
when both declarations supply them; absence in one declaration is not inequality.
Its `getBuiltinIteratorReturnType` selects `undefined` under effective
`strictBuiltinIteratorReturn`, otherwise `any`. These observations are not permission
to admit the whole group without proving generic/member compatibility or to borrow
the namespace namesake. Subsequent covariant/recursive iterator heritage and complete
structural callable inference still need their own proof after these barriers.

Cross-head structural inference, complete source-callable member projection and
relations, iterator generic obligations, exact constructor selected-origin/order
labels and callback negation remain open. The frozen manifest does not record
compiler source binding order: do not invent order from physical row or file IDs.
The broader multilingual correctness, IDE, benchmarking and flow roadmap is unchanged.

## Verification closeout (2026-09-08)

- Selected library suite: **2,128 passed**, zero failed, 32 ignored, 5,622 filtered.
- Seven integrations passed: TypeScript/JavaScript/Rust resolution corpora and
  per-file/per-package manifest context tests.
- Pinned compiler initializers: 54 cases, 38 exact result types, 38 constructor
  signature-presence checks, 14 diagnostic negatives, four legal abstentions.
  Two exact result labels belong to legal abstentions; 36 are supported positives.
- Pinned compiler overloads: 47 cases, 41 exact selected signatures/result types,
  six diagnostic negatives, two legal abstentions.
- Pinned compiler operator syntax: 12 cases, 19 operator nodes, 31 generic targets,
  zero diagnostics.
- Final manual frozen-source constructor/protocol probe passed. It is diagnostic,
  not an exact selected-constructor oracle or a complete cascade success assertion.
- Native whole-worktree audit: 335 changed production files, zero file-budget
  violations; 129 scoped resolver/type files, zero added string-identity patterns.
  `git -c core.safecrlf=false diff --check` passed. Native tooling was used because
  the BearWisdom MCP lookup tools are unavailable in this session.

The array change exposes no new public consumer API. Earlier sibling compilation
checks remain pending dependency/lockfile compatibility; no sibling files changed.
No broad 99%/99.9% milestone is completed by this bounded closeout.
