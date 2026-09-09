# Qualified trait calls — source-ID contract

1. [profile data] Describe bracketed qualified heads and their receiver/trait CST fields in namespace forms.
2. [generic] Capture each qualified selector, root span, caller slot, explicit Self recipe and trait recipe before resolution.
3. [generic] Persist recipes by source binding/declaration IDs and materialize them with the existing configured module graph.
4. [ingestion] Structurally flatten Rust scoped callees; bracketed heads remain single source-addressed segments, never split strings.
5. [generic] Select only the explicitly bound trait's static member ID and prove its Self obligation with the existing solver.
6. [generic] Match explicit receiver argument zero against the receiver signature without dot-call auto-borrow; static functions consume no receiver.
7. [generic] Substitute method type arguments and ordinary arguments after separating the receiver; preserve exact return TypeIds through chains and locals.
8. [generic] An attested qualified-call failure is authoritative across outer fallbacks; unrelated namesakes cannot repair missing proof.
9. [verification] First gate the three existing compiler-labelled qualified misses; add generic/return/receiver/static/negative and cold/edit/delete evidence.
10. [scope] Keep unsupported coercions, associated projections, higher-ranked/const constraints unknown; do not claim representative F0–F3 completion.

## Implemented contract

```rust
<Doc as Keep>::keep(p).touch();       // ✅ [generic] exact trait member ID → substituted Self TypeId → member ID
<Input as Choose>::choose(p, value); // ✅ [generic] explicit receiver slot is not ordinary parameter zero
<Input as Load<Doc>>::load().touch();// ✅ [generic] static trait function consumes no receiver argument
<Self as Save>::save(self);          // ✅ [generic] receiver value BindingId → physical method's receiver TypeId
<Doc as Empty>::absent(p);           // ✅ [generic] missing trait member cannot select an inherent namesake
<Doc as Save>::save(&p);             // ⚠️ explicit borrow-expression arguments still need source region capture
```

`namespace_type_traits` captures exact Self/trait recipes and caller slots from
profile-provided CST fields. `module_trait_inputs` persists them; `trait_graph`
materializes configured source IDs. `trait_qualified_call` shares the existing
obligation solver and canonical signature substitution. `chain_qualified_call`
uses stamped selector operands, not the reference's legacy display arguments.
The outer root is an authoritative failure barrier. A numeric site guard avoids
typing operands speculatively on ordinary dot calls.

Explicit method arguments are strict source recipes, too: the qualified path
never uses the legacy `segment_args` type-string fallback, and Unknown type
arguments cannot establish applicability.

Receiver values now have their own lexical BindingIds linked to physical method
slots, distinct from trait Self type parameters. No declaration is rediscovered by
the spelling `self`. The Rust extractor retains ordinary scoped-call module-prefix
metadata while recursively traversing CST paths instead of splitting type text.

Binding epoch 22 / extractor schema 42 invalidate earlier persisted inputs. Eight
new fixtures have 18 independently rustc-validated positive targets and one
diagnostic-backed negative; the three previously retained qualified misses now
have strict fresh/cold gates. Full implicit-Self UFCS, explicit borrow/coercion
expressions, inferred type placeholders, associated projections, higher-ranked
and const obligations remain open.

Semantic distinction: explicit paths disambiguate trait calls, while dot-call
receiver selection has its own adjustment ordering. Sources:
[Rust call expressions](https://doc.rust-lang.org/reference/expressions/call-expr.html),
[Rust method-call expressions](https://doc.rust-lang.org/reference/expressions/method-call-expr.html).

## Verification checkpoint

The targeted engine/profile/oracle cohort passes 2,131 tests (25 ignored).
All 12 selected integration tests pass; the Rust corpus was rerun after the final
strict type-argument boundary change. The provider test verifies exact IDs after
re-export retargeting, contract filtering, fresh-arena reload, implementation
removal with an inherent namesake present, and provider deletion.
The native file-budget/ID audit covers 244 changed production files without
violations; `git diff --check` passes.

Consumer compilation remains unverified: `--locked --offline` AlphaT checking
stops at the yanked `der ^0.8.0` dependency via `ureq`/`ort`; Lynx requires a lockfile
update. Both failures occur before compilation; neither consumer lock was changed.
