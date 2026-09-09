# Source-owned generic declaration augmentation

1. Fix the retained IteratorObject provider rejection before attempting shallow iterable structural inference.
2. Add compiler-labelled declaration/default/constraint fixtures and a failing default-to-return-to-member cascade first.
3. Keep interface groups private while reconciling source generic declarations; staging is not admission.
4. Compare parameter names through ingestion-owned IDs, retain declaration order/arity and reject unsupported parameter syntax.
5. Allocate one canonical owner parameter arena, including compiler-legal added defaulted parameters; each source declaration maps to its own prefix.
6. Collect supplied constraints/defaults separately from omitted ones; omissions cannot overwrite evidence or masquerade as conflicting types.
7. Preserve raw per-source signatures and derive effective canonical generic metadata for downstream materialization.
8. Prove supplied constraint/default agreement, legal arity, default ordering/cycles and default-to-constraint compatibility before public admission.
9. Feed the same proved metadata to local/global merge and interface heritage checks; retain negative, namesake, poisoned, portable, configuration and fresh/cold controls.
10. Remeasure the frozen real-project population; compiler-intrinsic typing, recursive/covariant iterator heritage, structural callable inference and constructor/callback work remain explicit until the whole cascade is verified.

```ts
interface Reader<T = Payload> { read(): T }
interface Reader<T> {}
reader.read().touch(); // ✅ [generic] proved effective default reaches the exact Payload member.
interface Bad<T = string> {}
interface Bad<T = number> {} // ✅ [generic] conflicting supplied defaults keep the group rejected.
```

The changes belong to generic source-signature/selected-view/merge infrastructure.
Source syntax recognition is ingestion data; no library/member spelling hook or
semantic format-and-re-resolve bridge is permitted. Effective compiler intrinsics
require a separate source identity/configuration contract and are not part of
default reconciliation itself.

Module-local groups also need private candidate identity from their lexical type
BindingId. The older workspace merge gate requires equal arity, so it cannot
supply a legal group that adds a defaulted parameter. Capture the complete
interface-only binding group as physical source rows, stage it before canonical
generic allocation, and let the same generic/member proof revoke invalid groups.
Keep this separate from the legacy workspace merge policy and from namesakes in
other scopes. Compiler reference: [generic parameter defaults](https://www.typescriptlang.org/docs/handbook/2/generics#generic-parameter-defaults).

## Implementation and compiler controls (2026-09-09)

`program_generic_groups` retains raw source signatures and collects effective
constraints/defaults without letting an omitted field overwrite a supplied one.
Canonical parameter IDs cover the longest legal declaration; each source part
receives its own prefix. Generic proof runs inside merge settlement, alongside
member and heritage checks, before a selected view can be published. Rejected
owners and their dependent declarations are rebuilt out of the view.

The proof checks source parameter names/order, supported syntax, duplicate names,
required arity, supplied type equality, declaration-local default ordering,
forward/self default references, circular constraints, and default compatibility
with effective constraints. Legal recursive nominal constraints stay distinct
from direct parameter cycles. Unsupported variance syntax stays an abstention.
The changes remain internal; binding epoch 65 and extractor schema 79 invalidate
older signature evidence.

The 30-case compiler cohort covers 14 supported positives, 15 diagnostic-backed
negatives and one legal abstention, each in script and module scope. Two return
cascades additionally carry eight exact compiler call-target checks across those
scopes. `verify_generic_augmentations.mjs` pins TypeScript 5.9.3 and checks both
symbol declaration and selected signature origins without reading engine results.

The first failing engine case was a module augmentation adding a defaulted
parameter: the legacy equal-arity gate left its declarations separate. The new
source-owned private groups fix it without changing the workspace merge policy.
A further negative showed that effective defaults could mask required-after-default
syntax in an original declaration. That now has a separate per-part check.

The older merge fixture for matching `T extends string` constraints now expects
admission; its source and compiler diagnostic labels are unchanged. The other
compiler-legal unsupported merge cases retain their barriers. The complete older
25-case compiler cohort still passes its diagnostics (eight positives, fourteen
negatives, three legal unsupported cases).

Portable extraction payloads omit flow by design. The call-cascade test rebuilds
identity through `contract_bindings::restore` before poisoning navigation metadata;
contract reduction is used for provider admission tests, where discarding body
call references is intentional. This avoids an empty or incorrectly initialized
test masquerading as a successful cache test.

## Real-source dependency and verification

The retained frozen-manifest probe now admits both physical `IteratorObject`
providers: the standard-library declaration (row 14167) and Node compatibility
augmentation (row 17126). The selected generic metadata retains the omitted/supplied
defaults, and its `Iterator<T, TReturn, TNext>` base canonicalizes successfully.

`ArrayIterator` (row 14170) remains rejected because its base includes the global
`BuiltinIteratorReturn` alias (row 14169), still captured as `Global(NameId(63))`
for the compiler intrinsic keyword. The similarly named NodeJS conditional alias
(row 17131) remains a separate declaration. No namesake substitution is allowed.

```ts
interface IteratorObject<T, TReturn = unknown, TNext = unknown> extends Iterator<T, TReturn, TNext> {}
interface IteratorObject<T, TReturn, TNext> {}
// ✅ [generic] both source providers now share a proved selected-view group.
type BuiltinIteratorReturn = intrinsic;
// ❌ [profile data + generic] source intrinsic identity/configuration is still absent.
const excludeSet = new Set(array2);
// ⚠️ [generic] readonly-array candidate proves Set<T>; iterable candidate stays unknown.
```

After intrinsic typing, recursive/covariant iterator heritage, complete callable
and structural inference, exact constructor origin/order and callback negation
still require their own evidence. No whole `difference<T>` cascade or 99% gate is
claimed by this change.

- Selected library suite: 2,133 passed, zero failed, 32 ignored, 5,622 filtered.
- Seven integrations passed across TypeScript/JavaScript/Rust resolution corpora
  and per-file/per-package manifest context tests.
- Pinned generic augmentation verifier: 30 cases in two scope variants, eight
  exact call-target/selected-signature checks; all diagnostic labels verified.
- Existing merge compiler verifier: 25 cases passed with the newly supported
  matching-owner-constraint case explicitly reclassified.
- Real source-provenance probe passed; source/configuration hashes were verified.
- Native worktree audit: 336 production files, 130 scoped resolver/type files,
  zero file-budget violations and zero added string-identity patterns.
- `git -c core.safecrlf=false diff --check` passed. No public API changed.

## Frozen accuracy reports

The 187 supplied sources, 24 selected files, 843 compiler calls and 690 labelled
targets are unchanged. Whole-report deep comparisons, excluding only elapsed
time, match the preceding array-identity reports in both modes, including every
fresh/cold occurrence. This step produces **no measured recall gain**.

- Configured: 575 correct, 40 declaration-kind-only disagreements, 75 unresolved;
  83.3333% recall, 93.4959% strict precision, zero snapshot changes.
- Legacy: 557 correct, 40 kind-only disagreements, 93 unresolved; zero snapshot
  changes.
- Configured report: `resolution-documents/2026-09-09-query-core-generic-augmentation-program-report.json`,
  SHA-256 `c41cc2adde8cde23f5cf0c3aea36395a0968eee1c37978e8f521a4eb1cbab04b`.
- Legacy report: `resolution-documents/2026-09-09-query-core-generic-augmentation-legacy-report.json`,
  SHA-256 `9dd3eb66ed961cce09abeedf44f1870a7a4a6b66db4eceea4cd859c2c967cce0`.
- Compiler manifest SHA-256 remains
  `44158824a3f91fdeda2d5e5da8a422054e178c76de29857c2f855ffe36a5110c`.

Elapsed times were 54,633 ms configured and 45,252 ms legacy. These are diagnostic
timings, not a controlled performance benchmark. Older reports and compiler
labels were preserved. The roadmap adds a completed generic-reconciliation child
and a pending intrinsic/iterator child; their larger parent remains unchecked.
