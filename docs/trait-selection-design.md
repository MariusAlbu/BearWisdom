# ID-bound trait selection

1. [generic] Compile retained trait headers, receiver recipes and obligations into numeric declaration/type indexes.
2. [generic] Give trait Self a distinct generic identity; bind implementation Self through its receiver recipe, never a namesake nominal.
3. [generic] Reuse the retained signature materializer with source BindingId overrides for Self and implementation parameters.
4. [profile data] Recognize abstract method signature owners through the existing trait member forms; add no runtime language-name branch.
5. [generic] Share parent-linked trait availability frames and preserve incomplete providers and competing imported identities.
6. [generic] Enumerate receiver candidates in order; compare inherent and applicable trait methods at each candidate, not in separate global passes.
7. [generic] Prove implementation bounds and scoped generic assumptions with bounded, cycle-aware ID traversal; unknown is not success.
8. [generic] Carry the selected static declaration, receiver adjustment and GenericParamId environment into call and return substitution.
9. [generic] Keep qualified-call binding, richer reborrow/outlives and configured external completeness explicit until independently verified.
10. [evidence] Start with the retained compiler-labelled precedence failure, then gate trait return cascades, negatives, provider changes and fresh/cold snapshots.

## Verified implementation checkpoint — 2026-09-07

`trait_graph` now compiles the persisted source contracts, and `trait_selection`
consumes them in the real chain walker. Each receiver candidate is checked against
inherent methods, scoped generic bounds and applicable visible traits. The selected
result carries its static declaration ID, adjusted receiver TypeId and GenericParamId
substitution environment into `chain_bound_method`; no method name is passed to that
selection API. MemberNameIds are installed once per file environment from source selectors.

`trait_obligations` checks supported exact type/trait applications and recursive impl
bounds with a bounded, cycle-aware proof. Method-specific where bounds are not dropped.
Unknown types/regions cannot prove themselves, including through an assumed bound.
Trait Self is a distinct canonical parameter; impl Self is the actual receiver recipe.
Source receiver signatures include abstract methods and owned Self. Owned Self does
not create a borrow region for output elision: ordinary input regions remain eligible.

```rust
fn f<T: Save>(p: T) { p.save(); } // ✅ [generic] scoped obligation → static trait declaration ID
let doc = wrapper.get();         // ✅ [generic] supported impl obligations + trait argument IDs → concrete return
doc.touch();                     // ✅ [generic] return TypeId preserves the correct namesake declaration
p.duplicate().touch();           // ✅ [generic] trait Self substitution carries implementor identity
<Doc as Save>::save(&p);          // ❌ [generic] qualified trait source-call binding remains open
```

The retained receiver-precedence wrong target is corrected. On the unchanged earlier
39-case diagnostic cohort, correct positive targets rose from 66/83 to 78/83.
Six additional independently labelled cases contribute eight correct positives and
one correctly rejected negative. Combined: 86/91 positives, five unresolved, zero
wrong targets, 11/11 correct negatives, all 102 labelled sites extracted, and fresh/cold
equality. A strict per-occurrence gate prevents corrected historical misses from
falling back to their older observation baseline. Historical labels/snapshots are unchanged.

The external-package integration regression exposed missing configured module evidence,
not a missing method. [Registry module ingestion](registry-module-binding-design.md)
now supplies that evidence; the method selector's incomplete-provider barrier remains.
Receiver head attestation preserves every original reference layer and application
argument, and refuses to stamp a projected pointee ID onto an unrelated wrapper head.
Tuple/array element hops update the exact receiver carried into the next method call.

Verification: 2,124 selected core/profile tests and all 12 selected integration tests
passed; pinned rustc verified 45 cases / 91 positive targets / 11 diagnostic negatives;
all 12 compiler-adapter tests passed. Binding epoch 21 / extraction schema 41.
AlphaT/Lynx consumer checks stopped before compilation on existing dependency/lockfile
constraints (yanked der 0.8.0 through ort/ureq; required Lynx lockfile update).

Still open: qualified trait calls (three current diagnostic misses), match payload
provenance (two), full autoderef/unsizing/coercion and reborrow/outlives rules,
associated/higher-ranked/const obligations, cfg/features, complete source/module
instances, and broader per-language semantics. This is not a representative Rust or
global 99% result. F0–F3 remain open.
