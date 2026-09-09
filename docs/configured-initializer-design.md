# Source-owned configured initializer values

1. Generic engine work: source expression recipes, selected-program value lookup, full constructor signatures and dependent field materialization; profile data identifies construction syntax, with no language/API-name hook.
2. Capture initializer declaration/signature owners and exact expression, callee and argument sites while source is available; preserve rowless field signatures and lexical value/type domains.
3. Lower callee/value paths once through BindingIds and source/module declaration IDs; do not infer a constructor result from a same-spelled type declaration.
4. Persist source-owned expression/type recipes, not workspace TypeIds or display signatures, including explicit generic arguments and authoritative unsupported expressions.
5. Gather full construct signatures from the selected runtime value's type and class constructor inventory; retain optional/rest, generic constraints/defaults, abstract/access and source origins.
6. Prove applicability and generic substitution before using a result; unknown competitors or conflicting applicable yields remain barriers, never an arbitrary first overload.
7. Evaluate initializer dependencies with the existing bounded private query solver and cycle handling; explicit field annotations take precedence and unavailable providers cannot supply fallback values.
8. Feed inferred values into configured physical field slots and row-independent signatures, then refresh dependent queries and signatures before publication; do not mutate shared workspace facts.
9. Verify compiler-backed constructor/value-head/field cascades, generic arguments, annotations, missing/invalid/ambiguous/access cases, provider retarget/edit/deletion and filtered/portable/cold evidence.
10. Re-evaluate every retained real-project occurrence against 548/690 configured and 557/690 legacy; track remaining expression/overload semantics explicitly instead of claiming general initializer or 99% completion.

```ts
interface Result<T> { read(): T }
interface Factory { new<T>(): Result<T> }
declare const Build: Factory;
interface Build<T> { wrong(): T }
class Holder<T> { value = new Build<T>() }
// ❌ [generic, before] configured field materialization has no initializer recipe.
// Target: runtime Build declaration → Factory construct signature → Result<T>, never type Build<T>.
```

## Implemented and verified (2026-09-08)

```ts
declare const Build: Factory;
interface Factory { new<T>(): Result<T> }
class Holder<T> { value = new Build<T>() }
// ✅ [generic] Source callee query → runtime declaration → full construct
// signature → GenericParamId substitution → configured field/signature value.

class Derived<T> extends Base<T> {}
// ✅ [generic] Implicit construction forwards source-bound base parameters;
// explicit no-base evidence distinguishes default construction from a missing provider.

// ⚠️ [generic] Different applicable overload yields, unproved argument relations,
// unsupported variadic rests and private/protected access contexts still abstain.
// No first-overload or same-spelled-type recovery is introduced.
```

Source capture now retains initializer trees, exact callee/operand sites, optional row owners, class constructor inventories, explicit superclass presence, constructor bodies and default-parameter optionality. Binding epoch 56 and extraction schema 72 invalidate older inputs. The runtime expression profile identifies syntax; there are no library-name hooks.

The private value-query solver memoizes initializer dependencies separately from field, signature, alias and query reads. Raw source construct signatures retain optional/rest parameters, generic constraints/defaults and source-owned generic IDs. Applicability uses bounded, tri-state type evidence: a failed conservative relation is not proof that a competitor is inapplicable. Class overload implementations are excluded when source overload declarations exist. Base constructor parameters are substituted into derived-class generic environments. Resource exhaustion, unknown providers, cycles and conflicting applicable results remain unavailable.

Materialization publishes unannotated inferred fields and rowless property signature results in the selected view. Existing annotations stay authoritative. Fixed-point evaluation now reruns source queries after materialized generic constraints even when repeated-property evidence itself has not changed. Shared workspace field facts and generic identities are not borrowed.

### Verification

- Original runtime-value/type-namesake regression failed before solver integration and passes afterward, retaining the caller's exact generic parameter ID.
- `verify_initializers.mjs` uses pinned TypeScript 5.9.3: 25 cases, 17 exact compiler result types/signatures, seven diagnostic-backed negatives and one legal overload-ordering abstention.
- Engine fixtures check every case fresh/cold, plus unchanged-consumer barrel retargeting, provider arity edits/deletion, shifted-arena portable/filtered rowless fields, poisoned display metadata, cyclic constructor results and distinct initializer results in overlapping programs sharing the same source file.
- Final selected library suite: 2,083 passed, 31 ignored, zero failures. Five selected integration targets: seven tests passed. The additional overlapping-program test followed the real reports; no production code changed after those reports.
- Native audits: 321 changed production files, zero file-budget violations; 121 scoped resolver/type files, zero added string-identity patterns. `git diff --check` passed.
- AlphaT/Lynx consumer compilation remains unverified because of the previously recorded dependency/lockfile incompatibility; neither sibling lockfile was modified.

### Retained real-project evidence

The original compiler manifest remains unchanged: SHA-256 `44158824a3f91fdeda2d5e5da8a422054e178c76de29857c2f855ffe36a5110c`; 187 supplied files, 24 selected files, 843 compiler calls, 690 labels and 153 unlabelled compiler calls.

- Configured report: `resolution-documents/2026-09-08-query-core-constructor-initializer-program-report.json`, SHA-256 `282bf8e4b0d277459268b1d04fb3be34b7ea848a61cf5b231ab0c6a02c4733a2`. Correct calls increase 548 → 571; unresolved decrease 102 → 79; 40 declaration-kind-only disagreements remain. Recall 82.7536231884%, strict precision 93.4533551555%. All 23 changed labelled sites are unresolved → correct, every prior correct site is preserved, fresh/cold occurrence records agree and no source-capture gaps appear.
- The gains include nine `listeners` collection calls across `Subscribable` and subclasses, plus fourteen calls through constructed manager instances. Both previously repaired QueryObserverOptions sites remain unchanged.
- Legacy report: `resolution-documents/2026-09-08-query-core-constructor-initializer-legacy-report.json`, SHA-256 `4778106a8a0d5bcb9bfde01d35944685f06984790422cbcedda5297b6119678b`. The entire preceding legacy report is unchanged except elapsed time: 557 correct, 40 kind-only disagreements, 93 unresolved, fresh/cold equal.
- Four unlabelled compiler calls remain unextracted, and three extractor-only sites remain unchanged. The labelled extraction rate is not full compiler-call extraction coverage. Neither report passes the representative 99% gate. Diagnostic elapsed times are not controlled performance benchmarks.

### Remaining scope

General call/arrow/object/array initializers, declaration-context literal widening, rich structural constructor values, full contextual/variance-aware inference, variadic array/rest evidence, complete overload ordering and caller-sensitive constructor access remain open. Broader mapped/distributive/key-equivalence semantics and newly introduced structural navigation origins also remain open. Rank the retained 79 misses by traced upstream cause before selecting the next cascade; do not infer priority from repeated member spellings.

The post-change trace artifact is `resolution-documents/2026-09-08-query-core-post-initializer-trace.json`, SHA-256 `80534c0b31e97a903679588e82c9e200f6b2eb2925aa49ce661201ab4019aa53`. It requests all 79 retained misses and contains 284 trace records; its entire evaluation report equals the constructor report except elapsed time. Adjacent-line, prepass and fresh/cold records are mixed, so 284 is not a count of distinct unresolved occurrences.

```ts
const observer = this.observers.find(x => x.shouldFetchOnWindowFocus());
// ❌ [generic] The receiver is typed, but member_selection rejects multiple
// source member IDs before argument/contextual overload evidence is considered.
// The pinned Array.find declarations include predicate and ordinary overloads.
observer?.refetch();
// ❌ [generic] No selected callable result reaches this dependent local.

export const environmentManager = (() => {
  return { isServer(): boolean { return isServerFn() } };
})();
// ❌ [generic] Configured initializer capture has no call/IIFE/object-return recipe;
// imported consumers consequently read an unknown root despite a real method body.
```

Other traced boundaries include structural callable properties (`TimeoutManager.#provider` is annotated `TimeoutProvider<any>`), intersections carrying pending-promise members, missing contextual callback parameters, and local narrowing. These are separate causes, not reasons to add member-name fallbacks. Before extending overload dispatch, add independent selected-signature evidence: the existing real-project manifest explicitly labels call-symbol navigation, not overload dispatch. Preserve callable-group navigation origins separately from overload candidates and their substituted yields.
