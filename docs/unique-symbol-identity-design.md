# Unique-symbol ownership and computed-key binding

1. Root cause: a captured `unique symbol` flag is not a type identity; selected signatures currently materialize it as unresolved.
2. Recognize unique annotations and their legal declaration-owner forms through syntax profile data, never an API-name list.
3. Represent the introducing declaration by a source span even when a property has no navigation row.
4. Mint unique identities inside a selected program/source instance; identical source offsets in another file/program cannot collide.
5. Preserve the introducing identity through source recipes and cold snapshots; never reconstruct it from the symbol's display spelling.
6. Keep source caches portable: program-owned unique TypeIds require source recapture, not serialization into unconfigured names.
7. Carry source-owned unique sites into configured signature allocation before materializing dependent signatures.
8. Verify legal/illegal owner forms independently against TypeScript and verify same-spelled/same-offset/filtered/cold isolation.
9. Follow with value-query and computed-key binding through the selected source value/member graph, including alias, import, cycle and ambiguity evidence.
10. Keep declaration-merging guards until compatibility is proved; source origin identity is not a license to admit rich merges or a 99% claim.

## Selected-program computed-key implementation

1. Capture named member value signatures independently of optional navigation rows, including negative optional/access/method evidence.
2. Lower computed roots to annotated binding, declaration, import binding or source-global IDs while lexical source ownership exists.
3. Intern member-name payloads once at configured-view construction; semantic selector traversal consumes only numeric keys.
4. Resolve named member values through source signature arenas after signature materialization, preserving static versus instance ownership.
5. Keep duplicate candidates, unsupported intermediate types and rejected declaration groups unresolved; never first-win.
6. Bind the terminal key only when its type is a unique symbol belonging to the selected program context.
7. Expose captured/unknown/bound key results by exact source span for subsequent rich-member compatibility work.
8. Test renamed imports, unchanged-consumer provider retargeting/deletion, rowless members, ambiguity, access and source/program isolation.
9. Verify compiler-owned unique targets independently and retain poisoned-display and portable/fresh-arena cold controls.
10. Do not enable rich merges until key and signature compatibility are both proved; retain the full roadmap and real-project gates.

## Implemented identity path

```ts
declare const tag: unique symbol; // ✅ [profile data] legal owner -> exact declaration source span
interface Keys { readonly token: unique symbol; } // ✅ [generic] row-independent source signature
declare const keys: Keys;
interface Box<T> { value: T; }
declare const box: Box<Keys>;
declare class Factory { static readonly token: unique symbol; }
interface Uses {
  [tag](): void;             // ✅ [generic] annotated BindingId -> selected unique TypeId
  [keys.token](): void;      // ✅ [generic] global value ID -> nominal owner + MemberNameId -> source signature
  [box.value.token](): void; // ✅ [generic] receiver GenericParamId substitution -> same unique origin
  [Factory.token](): void;   // ✅ [generic] class value identity -> static member, not an instance member
}
type Tag = typeof tag;      // ❌ [generic] configured source value-query recipes still missing
```

The successful markers describe computed **declaration-key identity**, not a new
computed-call resolver or permission to merge arbitrary interfaces. The selected
lookup exposes uncaptured / captured-unknown / bound results by exact key span.
Member spellings are interned once at view construction. Selector traversal uses
numeric source, declaration, member-name, generic and type IDs.

`UniqueSymbol` contains program context, source ordinal and declaration span.
Same-spelled declarations, identical bytes in separate files, and separate programs
remain distinct. Hydration remints unique and nominal contexts together. Portable
external caches recapture source origins instead of restoring a configured unique
identity as an unconfigured type. The original unique type is preserved when its
apparent receiver is projected to the source-bound symbol wrapper.

Named member inputs retain static/instance and readability evidence independently
of navigation rows. Optional, private/protected, method-only, duplicate and missing
members cannot supply an exact key through a fallback. Duplicate declarations need
compatibility proof even if provisional types agree. Repeated readonly unique
properties can legally share a compiler symbol across merged interfaces: separate
source-origin IDs must not be mistaken for a final merge decision.

## Verification, 2026-09-08

- The retained failing-before annotation regression produced `Unknown` (Cargo 85130).
- Initial computed-key compilation exposed four trait/option plumbing errors plus
  a pre-existing unfinished transparent-node cursor lifetime error (Cargo 43267).
  These were corrected without changing semantic assertions.
- Cargo 88321: eight focused tests passed; Cargo 56387: all 12 focused tests passed
  after adding import edits/deletion, rowless portable/cold, program isolation and
  independently labelled computed-key targets.
- TypeScript 5.9.3 verifies 15 unique-owner cases: eight declaration origins and
  seven diagnostic-backed negatives. Both Rust TypeScript/TSX source controls pass.
- A separate compiler verifier checks four shared multi-file fixtures and seven
  exact computed-expression unique declaration targets with zero diagnostics,
  including renamed imports, nested generics and non-ASCII source offsets.
- Cargo 19652: 2,002 selected library tests passed, 29 ignored, zero failures.
- Cargo 29573: all seven selected integration tests passed.
- Binding epoch 45; extraction schema 65. All 25 touched production files satisfy
  the line budget: computed-key binder 136 lines, unique type 21, core types 688
  versus 730 at HEAD. `git diff --check` passes.

Public AlphaT/Lynx consumer compilation remains unverified because of the retained
locked dependency failures. No sibling source or lockfile was changed.

Real-project reports retain the exact 187-provider / 24-selected-file / 843-call /
690-label compiler manifest (SHA-256
`44158824a3f91fdeda2d5e5da8a422054e178c76de29857c2f855ffe36a5110c`).
Both entire JSON reports are deep-equal to the source-operator reports after
excluding only `elapsed_ms`, including every occurrence and cold result:

- Configured: `resolution-documents/2026-09-08-query-core-unique-key-program-report.json`,
  SHA-256 `0c007df2511f1339ddfbc1cf3b3d4cdf0f18a55159c0aee99033c05e154eb4bb`;
  436 correct, 40 kind-only disagreements, 214 unresolved, zero configured-source
  gaps and zero snapshot changes. Cargo 95538 completed successfully.
- Legacy: `resolution-documents/2026-09-08-query-core-unique-key-legacy-report.json`,
  SHA-256 `15a170770794c6b43e8ebea2141cd8e0db05e39abdd9983dbd8dfa7a78dad521`;
  557 correct, 40 kind-only disagreements, 93 unresolved and zero snapshot changes.
  Standalone current executable 1508 completed successfully.
- Recall remains 63.1884% configured and 80.7246% legacy. Neither correctness gate
  passes. Timings (34,512/48,139 ms) are uncontrolled and not benchmark evidence.

No production code changed after final library/integration verification.

## Remaining work

Source value/type queries, namespace-valued key roots, initializer-inferred unique
values, general expression wrappers and literal/computed selectors remain open.
The named-member path does not establish generic constraints, complete interface
inheritance, structural object identity or all accessibility rules. Rich merge
compatibility must consume bound keys and complete source types before relaxing
existing rejection guards. Then materialize initializer-derived field/value types
and remeasure the retained Set/Promise/Array/listeners cascades. The full F0–F3
objective and representative 99%/99.9% gates remain open.

There is also a staging dependency to address before real stdlib merges can be
proved: `program_graph::bind_group` rejects non-plain groups before configured
signatures and keys are materialized. Compatibility checking needs an isolated
candidate-binding phase, with only proved groups promoted to the public view.
Simply deleting the early guard would expose unproved bindings. Source-origin
unique IDs also need explicit canonical member identity for legally merged
properties; treating different origins as automatically incompatible is not the
compiler's final semantics.
