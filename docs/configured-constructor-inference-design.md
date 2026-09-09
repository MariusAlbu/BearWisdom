# Constructor argument inference and the retained callback cascade

1. Trace the retained `difference<T>` constructor's runtime value, source signatures and argument types before changing member lookup.
2. Keep constructor results attached to source initializer/declaration IDs; never manufacture a type from the callee's spelling.
3. Extend the shared configured argument inference engine, so constructors and ordinary overloads use the same evidence.
4. Model union inference with private candidate bindings; distinguish no inference, contradictory evidence and unsupported evidence.
5. Preserve generic owner identities and constraints, including caller-owned generic parameters and nested nullable arguments.
6. Structural/readonly/iterable inference must use attested member/signature/key surfaces, not an Array/Set/Iterable name switch.
7. Add source callback negation through grammar profile data and a generic expression recipe only after tracing its operand types.
8. Publish callback contexts only after selection; preserve source origins and final applicability checks for every chosen constructor/call.
9. Pin independent compiler result/signature labels, negative counterexamples, complete downstream cascades and portable/cold/edit checks.
10. Remeasure every retained Query Core occurrence; leave the roadmap cascade open until the real constructor, callback and filter all work.

## Starting evidence

```ts
function difference<T>(array1: Array<T>, array2: Array<T>): Array<T> {
  const excludeSet = new Set(array2); // ❌ [generic] retained trace has a bare Set; constructor inference supports matching Apply heads only.
  return array1.filter(x => !excludeSet.has(x)); // ❌ [generic] upstream generic receiver and callback negation need independent proof.
}
```

The retained baseline is 575/690 correct, 40 declaration-kind disagreements and 75
unresolved calls. No gain or complete constructor/array semantics is assumed.
`program_constructor_arguments::infer` currently does not traverse union parameter
types. The supplied constructor declarations also contain different readonly-array
and iterable parameter heads; union support alone must not be claimed to close them.

## Source class-member projection (design before integration)

1. The poisoned portable constructor cascade fails at `holder.value`, before reading the newly inferred field type.
2. `View::materialize` currently restores class member rows from physical symbol display names.
3. Capture class member projections once from existing source class inventories, canonical owner IDs and source NameIds.
4. Intern source spellings only at this capture boundary to obtain the selected view's MemberNameIds.
5. Preserve all same-name source rows and rowless entries, rather than picking the first physical declaration.
6. Replace each captured class owner's physical member-name rows during every view materialization round.
7. Never retain a fabricated display-name entry alongside its source-attested identity.
8. Leave member access, signatures, generic substitution and inheritance selection to their existing ID-based proofs.
9. Verify poisoned declaration metadata, source-provider replacement/deletion, cold reload and unchanged interface behavior.
10. Keep this as shared engine work over existing class inventories: no new language hooks or library-name rules.

## Follow-through: selected source member identities

1. The strengthened regression corrupts every extracted symbol name, qualified name and signature, not only call operands.
2. The constructor field retains its correct `Result<Payload>` TypeId and the class field selector now reaches its source row.
3. The next failure is the source `Result.read` interface member, whose standalone physical member index still uses display metadata.
4. Generalize the same source projection to captured named class AND interface members; neither kind may recover identity from display names.
5. Rename the class-only projection module to reflect its shared source-member responsibility, retaining its sibling tests.
6. Preserve every attested candidate and authoritative rowless declaration; rich merging and inheritance still require their separate compatibility proofs.
7. Materialize the per-source NameId-to-selected-MemberNameId mapping once with the projection; file selectors consume that numeric mapping.
8. Revoke old physical display aliases only for rows covered by source evidence; do not silently erase still-unmigrated member forms.
9. Assert exact field and interface declaration IDs before checking both local and inline downstream calls, including portable and cold reloads.
10. Run the constructor, member, overload and snapshot gates and the fixed real-project oracle before claiming this boundary is complete.

```ts
const item = holder.value.read(); // ⚠️ [generic] constructor value and class selector are typed; standalone interface member rows still need source projection.
item.touch();                    // ❌ [generic] depends on the preceding source member binding and generic return publication.
holder.value.read().touch();      // ❌ [generic] the same missing source member must be repaired without display-name recovery.
```

## Implemented and directly verified (epoch 63 / schema 77)

`program_constructor_arguments` now canonicalizes both operands and traverses
inference-bearing union positions using the signature's GenericParamIds. Exact
and base-literal matches precede same-application-head matches. Both matched
source and target constituents are removed before fallback inference. An
unmatched naked generic can receive lower-priority inference from the original
source when matching consumed every source constituent; ordinary argument
candidates outrank this fallback regardless of argument order. Union positions
preserve top-level literal-widening context; nested applications do not.

Inference occurrences are distinct from binding-map mutations: repeated evidence
still counts even when it yields the same TypeId. Unsupported inference-bearing
shapes remain unknown. Different composite TypeIds are not evidence of structural
inequality, and general conflicting-candidate/variance inference remains open.
All final parameter, constraint, arity and result proofs still run; constructor
access and erased-callable barriers are unchanged. Shared call applicability
consumes this inference too; this is not a constructor-specific library hook.

`program_source_members` now captures named class and interface member rows from
source inventories and canonical owner IDs, preserves competing/rowless entries,
and removes covered physical display aliases in one index pass. It also lowers
each source NameId into the selected view's member-name arena. `FileLookup`
consumes that mapping directly instead of looking up selector spellings in the
workspace arena. Materialization restores these projections each round. This
does not claim migration of every literal/computed/private or legacy member form.

The strengthened regression failed first at the class field, then at the
standalone interface method (`read`: no candidate instead of declaration row 6).
It now verifies exact source field/method IDs and both downstream `touch` sites
with every extracted symbol name/qualified name/signature and call operand
corrupted, through a shifted portable arena and a cold database snapshot.
Provider retargeting, invalidating edits and deletion pass for both explicit
constructor arguments and nullable inferred constructor arguments.

```ts
const item = holder.value.read(); // ✅ [generic] captured member IDs, inferred Result<Payload>, and source generic return substitution.
item.touch();                    // ✅ [generic] BindingId-owned result publication reaches the exact Payload member ID.
holder.value.read().touch();      // ✅ [generic] the same source target survives inline continuation and cold reload.
```

Pinned TypeScript 5.9.3 verifies 42 constructor cases: 31 exact result types and
constructor-signature-presence checks, nine diagnostic-backed negatives and
three legal abstentions. One exact result label belongs to a legal abstention;
there are 30 supported positive engine cases. Constructor signature **presence**
is not an exact selected-declaration oracle. Two provisional new fallback labels
were falsified by the compiler and corrected before being accepted; existing
frozen labels were not changed. The verifier compares cyclic compiler Type
objects by identity but formats only type strings on failure, avoiding the
assertion library's unbounded object-diff allocation.

Selected library verification: 2,120 passed, zero failed, 32 ignored. The native
whole-worktree gates found zero file-budget violations across 333 changed
production files and no added string-identity patterns across 128 scoped files.

## Retained real-project constructor provenance

The manual frozen-manifest probe verifies the 187 supplied inputs before
inspecting `queriesObserver.ts` initializer span 446..474. The runtime callee
binds to the selected `SetConstructor` declaration and `array2` binds to the
caller's `Array<T>` application. Neither is an unbound name.

```ts
new Set(array2); // ❌ [generic] source construction is bound; complete argument inference/proof is not yet available.
// lib.es2015.collection.d.ts, signature span 4246..4297:
// ❌ [generic] readonly T[] | null: program_structural_ops rejects TypeOperator::Readonly before inference.
// lib.es2015.iterable.d.ts, signature span 6641..6687:
// ❌ [generic] Iterable<T> | null: inference across different structural application heads is unsupported.
array1.filter(x => !excludeSet.has(x)); // ❌ [generic] also needs source callback negation after the constructor receiver is proved.
```

Readonly support must retain attested array/tuple syntax and selected global
intrinsic identity, not strip the operator or match an Array spelling. Constructor
candidate ordering/selected origins remain separate from the implemented named
method overload ordering. The full `difference<T>` cascade stays open.

## Verification and fixed-cohort recapture

- Four targeted constructor/source-member/provider-lifecycle tests passed.
- Seven integrations passed: per-file manifest (two), per-package context (two),
  and TypeScript, JavaScript and Rust resolution corpora (one each).
- The unchanged pinned overload cohort passed all 47 compiler cases: 41 exact
  selected signatures/result types, six diagnostic negatives, two legal abstentions.
- The manual frozen-input constructor provenance probe passed on the final build
  and retained both readonly/iterable applicability barriers described above.
- `2026-09-08-query-core-constructor-union-verified-program-report.json`:
  575 correct, 40 kind-only disagreements, 75 unresolved; 83.3333% recall,
  93.4959% strict precision, zero snapshot changes and gate ineligible. The whole
  report equals the prior overload-order report except elapsed time, not merely
  its aggregate counts. SHA-256:
  `4792bcd6a0c9bc2c1e9dbc187e6f45a26570bbcfce7156b15ed9d6ac1e008639`.
- The fixed manifest is unchanged, SHA-256
  `44158824a3f91fdeda2d5e5da8a422054e178c76de29857c2f855ffe36a5110c`.
  No supply, exclusions, labels or compiler options were relaxed.

The configured run overlapped the separate manual provenance probe. Its 64,684ms
elapsed time is diagnostic, not controlled performance evidence.

Further compiler-source inspection identifies the next array mechanism:
TypeScript 5.9.3 `inferFromObjectTypes` (`typescript.js:73578`) pairs type arguments
when reference targets are identical **or both targets are attested array types**.
Array/tuple-to-array inference also uses index types. A future source-bound array
role should follow the selected-global identity approach already used by
`intrinsic_members`, preserve mutable/readonly directionality and tuple shape,
and retain negative namesake/configuration tests. Merely stripping `Readonly` or
treating arbitrary indexable objects as arrays would not implement that rule.

Legacy comparison completed with the final compiled example:
`2026-09-08-query-core-constructor-union-verified-legacy-report.json`, 557 correct,
40 kind-only disagreements, 93 unresolved and zero snapshot changes. Its entire
report equals the prior verified legacy report except elapsed time (52,179ms).
SHA-256: `4e87c81afc816cc0278f3e34326a861bb2b7f0d8fde7d6ce0b7203e6141bfad1`.
Both new reports use create-new output paths; prior reports were preserved.
